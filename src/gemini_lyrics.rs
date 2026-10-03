//! Find a song's lyrics: Gemini searches Google, the app fetches the pages it found, and Gemini copies the lyrics out.
//!
//! Why not simply ask Gemini (with Google Search) for the lyrics: its copyright filter cut most such answers off
//! (finish reason RECITATION, measured at about 4 in 5 on a 1968 Hebrew song), and neither a unique prompt nor a higher
//! temperature (Google's troubleshooting advice) helped. Copying lyrics out of a page that is given to it is not
//! blocked (20 of 20) and gives the same text every time, so the steps are:
//!   1. Gemini, with Google Search, is asked which websites have the lyrics (a question it doesn't answer by reciting).
//!      The pages it found come back as links in the answer's grounding metadata.
//!   2. The app downloads those pages and turns them into plain text (no site-specific parsing).
//!   3. Gemini, with no tools, is given the pages and returns the lyrics from the best one.

use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::gemini::{self, Client, GeminiError};
use crate::html_text;
use crate::log;
use crate::py;

/// Grounding with Google Search is billed per search query Gemini runs, on top of the tokens: the first 5,000 queries
/// a month (shared by all Gemini 3.x models) are free, then $14 per 1,000. https://ai.google.dev/gemini-api/docs/pricing
const SEARCH_PRICE_PER_1000: f64 = 14.0;
const SEARCH_FREE_PER_MONTH: i64 = 5000;

/// How many of the found pages are downloaded and shown to Gemini, and how much of each
const MAX_PAGES: usize = 4;
const MAX_PAGE_CHARS: usize = 20_000;
const FETCH_TIMEOUT_SECONDS: u64 = 15;
/// Video sites come up in the search but their pages have no lyrics text
const SKIPPED_HOSTS: [&str; 2] = ["youtube.com", "youtu.be"];
/// Some lyrics sites refuse requests that don't look like a browser
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Safari/537.36";
const ACCEPT_LANGUAGE: &str = "he,en;q=0.8";

// ---------------------------------------------------------------- step 1: search

pub fn build_search_prompt(title: &str, artist: &str) -> String {
    // Asking for the pages' exact titles makes Gemini actually search: asked for the websites' names, it sometimes
    // (4 in 36) listed the usual lyrics sites from memory, which gives no links. Measured 72 of 72 with links.
    format!(
        "Search Google for web pages that show the full lyrics of the song \"{}\" by {}. Reply only with the exact titles of the pages in the search results.",
        title, artist
    )
}

struct Found {
    links: Vec<(String, String)>,
    queries: Vec<String>,
    cost: Map<String, Value>,
}

async fn search_pages(client: &Client, title: &str, artist: &str) -> Result<Found, GeminiError> {
    let body = json!({
        "contents": [{"parts": [{"text": build_search_prompt(title, artist)}], "role": "user"}],
        "tools": [{"googleSearch": {}}],
        "generationConfig": {"thinkingConfig": gemini::thinking_config()},
    });
    let response = client.generate_content(&body).await?;
    let metadata = response.get("candidates").and_then(|c| c.get(0)).and_then(|c| c.get("groundingMetadata"));
    let mut links = Vec::new();
    if let Some(Value::Array(chunks)) = metadata.and_then(|m| m.get("groundingChunks")) {
        for chunk in chunks {
            if let Some(web) = chunk.get("web") {
                let uri = web.get("uri").and_then(Value::as_str).unwrap_or("");
                if !uri.is_empty() {
                    links.push((web.get("title").and_then(Value::as_str).unwrap_or("").to_string(), uri.to_string()));
                }
            }
        }
    }
    let queries = match metadata.and_then(|m| m.get("webSearchQueries")) {
        Some(Value::Array(q)) => q.iter().filter_map(Value::as_str).map(String::from).collect(),
        _ => Vec::new(),
    };
    Ok(Found { links, queries, cost: gemini::compute_cost(response.get("usageMetadata").unwrap_or(&Value::Null)) })
}

// ---------------------------------------------------------------- step 2: fetch

fn is_skipped_host(host: &str) -> bool {
    SKIPPED_HOSTS.iter().any(|h| host == *h || host.ends_with(&format!(".{}", h)))
}

/// The charset parameter of a Content-Type header (as httpx's response.charset_encoding).
fn header_charset(content_type: &str) -> Option<String> {
    content_type.split(';').skip(1).find_map(|p| {
        let (k, v) = p.split_once('=')?;
        (k.trim().eq_ignore_ascii_case("charset")).then(|| v.trim().trim_matches('"').trim_matches('\'').to_lowercase())
    })
}

/// Download a page (following Google's redirect link) as plain text; None if it can't be read.
async fn fetch_page(http: &reqwest::Client, url: &str) -> Option<(String, String)> {
    let response = http.get(url).send().await.ok()?;
    let final_url = response.url().to_string();
    let host = response.url().host_str().unwrap_or("").to_lowercase();
    if response.status().as_u16() != 200 || is_skipped_host(&host) {
        return None;
    }
    let content_type = response.headers().get("content-type").map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned());
    if !content_type.as_deref().unwrap_or("html").contains("html") {
        return None;
    }
    let charset = content_type.as_deref().and_then(header_charset);
    let bytes = response.bytes().await.ok()?;
    let text = html_text::html_to_text(&html_text::decode_page(&bytes, charset.as_deref()));
    if text.is_empty() {
        return None;
    }
    Some((final_url, text.chars().take(MAX_PAGE_CHARS).collect()))
}

/// Step 2. The first MAX_PAGES pages that could be read, in the search's order.
async fn fetch_pages(links: &[(String, String)]) -> Vec<(String, String)> {
    let http = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .default_headers({
            let mut h = reqwest::header::HeaderMap::new();
            h.insert("Accept-Language", reqwest::header::HeaderValue::from_static(ACCEPT_LANGUAGE));
            h.insert("Accept", reqwest::header::HeaderValue::from_static("*/*"));
            h
        })
        .redirect(reqwest::redirect::Policy::limited(20))
        .referer(false)
        .connect_timeout(Duration::from_secs(FETCH_TIMEOUT_SECONDS))
        .read_timeout(Duration::from_secs(FETCH_TIMEOUT_SECONDS))
        .build()
        .expect("HTTP client");
    let candidates: Vec<&String> = links.iter().filter(|(site, _)| !is_skipped_host(site)).map(|(_, url)| url).take(MAX_PAGES * 2).collect();
    let pages = futures_util::future::join_all(candidates.iter().map(|url| fetch_page(&http, url))).await;
    pages.into_iter().flatten().take(MAX_PAGES).collect()
}

// ---------------------------------------------------------------- step 3: extract

fn extract_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "page": {"type": "integer", "description": "Number of the page the lyrics were taken from, or 0 if no page has them"},
            "lyrics": {"type": "string", "description": "The lyrics, or an empty string"},
        },
        "required": ["page", "lyrics"],
    })
}

pub fn build_extract_prompt(title: &str, artist: &str, pages: &[(String, String)]) -> String {
    let listing: Vec<String> = pages.iter().enumerate().map(|(i, (url, text))| format!("<page number=\"{}\" url=\"{}\">\n{}\n</page>", i + 1, url, text)).collect();
    format!(
        "Below are web pages found for the song \"{}\" by {}. Pick the page with the most complete and \
reliable lyrics of this song and copy its lyrics exactly as written there.

The lyrics must contain no additional text:
- no title, artist, credits, introduction, explanation or notes
- no section labels such as [Chorus] or \"Verse 1\", and no chords
- a single blank line between verses (stanzas); within a verse, one line per sung line

If none of the pages has the lyrics of this song, answer with page 0 and empty lyrics.

{}",
        title,
        artist,
        listing.join("\n\n")
    )
}

/// Tidy line endings and blank lines.
pub fn clean_lyrics(raw: &str) -> String {
    let normalized = raw.replace("\r\n", "\n");
    let joined = normalized.split('\n').map(|l| l.trim_end_matches(py::is_space)).collect::<Vec<_>>().join("\n");
    let re = regex::Regex::new(r"\n{3,}").unwrap();
    py::strip(&re.replace_all(&joined, "\n\n")).to_string()
}

struct Extracted {
    lyrics: String,
    source_url: Option<String>,
    cost: Map<String, Value>,
}

async fn extract_lyrics(client: &Client, title: &str, artist: &str, pages: &[(String, String)]) -> Result<Extracted, GeminiError> {
    let body = json!({
        "contents": [{"parts": [{"text": build_extract_prompt(title, artist, pages)}], "role": "user"}],
        "generationConfig": {
            "responseMimeType": "application/json",
            "responseJsonSchema": extract_schema(),
            "thinkingConfig": gemini::thinking_config(),
        },
    });
    let response = client.generate_content(&body).await?;
    let cost = gemini::compute_cost(response.get("usageMetadata").unwrap_or(&Value::Null));
    let text = gemini::response_text(&response).filter(|t| !t.is_empty());
    let Some(text) = text else {
        let reason = response.get("candidates").and_then(|c| c.get(0)).and_then(|c| c.get("finishReason")).and_then(Value::as_str).unwrap_or("None");
        log::warn(&format!("find_lyrics: extraction for {:?} by {:?} gave no answer (finish reason {})", title, artist, reason));
        return Err(GeminiError(format!("Gemini לא החזיר את המילים (סיבה: {}). אפשר להדביק אותן ידנית.", reason)));
    };
    let bad = || GeminiError("התשובה של Gemini לא הייתה בפורמט הצפוי. נסה שוב.".into());
    let answer: Value = serde_json::from_str(&text).map_err(|_| bad())?;
    let answer = answer.as_object().ok_or_else(bad)?;
    let page_number = match answer.get("page") {
        v if !py::truthy(v) => 0,
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f.trunc() as i64)).ok_or_else(bad)?,
        Some(Value::String(s)) => py::strip(s).parse().map_err(|_| bad())?,
        Some(Value::Bool(b)) => *b as i64,
        _ => return Err(bad()),
    };
    let lyrics = match answer.get("lyrics") {
        v if !py::truthy(v) => String::new(),
        Some(Value::String(s)) => clean_lyrics(s),
        _ => return Err(bad()),
    };
    let source = if page_number >= 1 && page_number as usize <= pages.len() { Some(pages[page_number as usize - 1].0.clone()) } else { None };
    Ok(Extracted { lyrics: if source.is_some() { lyrics } else { String::new() }, source_url: source, cost })
}

// ---------------------------------------------------------------- together

/// Sum the costs of two requests (the prices are the same in both).
fn add_costs(mut total: Map<String, Value>, cost: &Map<String, Value>) -> Map<String, Value> {
    for key in ["input_tokens", "output_tokens", "thinking_tokens"] {
        let v = total[key].as_i64().unwrap_or(0) + cost[key].as_i64().unwrap_or(0);
        total.insert(key.into(), json!(v));
    }
    for key in ["input_cost_usd", "output_cost_usd", "total_cost_usd"] {
        let v = total[key].as_f64().unwrap_or(0.0) + cost[key].as_f64().unwrap_or(0.0);
        total.insert(key.into(), json!(v));
    }
    let mut modalities = total["input_tokens_by_modality"].as_object().cloned().unwrap_or_default();
    if let Some(m) = cost["input_tokens_by_modality"].as_object() {
        for (name, count) in m {
            let v = modalities.get(name).and_then(Value::as_i64).unwrap_or(0) + count.as_i64().unwrap_or(0);
            modalities.insert(name.clone(), json!(v));
        }
    }
    total.insert("input_tokens_by_modality".into(), Value::Object(modalities));
    total
}

/// Find a song's lyrics on the web. Returns {"lyrics", "source_url", "search_queries", "cost"}.
pub async fn find_lyrics(title: &str, artist: &str) -> Result<Map<String, Value>, GeminiError> {
    let (title, artist) = (py::strip(title), py::strip(artist));
    if title.is_empty() || artist.is_empty() {
        return Err(GeminiError("יש למלא גם שם שיר וגם שם אמן כדי לחפש מילים.".into()));
    }
    let client = Client::new(gemini::api_key()?);
    let found = search_pages(&client, title, artist).await?;
    let pages = fetch_pages(&found.links).await;
    log::info(&format!(
        "find_lyrics: {:?} by {:?}: search found {} links, read {} pages: {:?}",
        title,
        artist,
        found.links.len(),
        pages.len(),
        pages.iter().map(|p| p.0.as_str()).collect::<Vec<_>>()
    ));
    if pages.is_empty() {
        return Err(GeminiError("לא נמצאו דפי מילים לשיר הזה. בדוק את שם השיר והאמן, או הדבק את המילים ידנית.".into()));
    }
    let result = extract_lyrics(&client, title, artist, &pages).await?;
    if result.lyrics.is_empty() {
        return Err(GeminiError("לא נמצאו מילים לשיר הזה בדפים שנמצאו. בדוק את שם השיר והאמן, או הדבק את המילים ידנית.".into()));
    }

    let mut cost = add_costs(found.cost, &result.cost);
    // List price: whether the searches are actually charged depends on the month's free allowance, which we can't see
    let search_cost = found.queries.len() as f64 * SEARCH_PRICE_PER_1000 / 1000.0;
    cost.insert("search_query_count".into(), json!(found.queries.len()));
    cost.insert("search_price_per_1000".into(), json!(SEARCH_PRICE_PER_1000));
    cost.insert("search_free_per_month".into(), json!(SEARCH_FREE_PER_MONTH));
    cost.insert("search_cost_usd".into(), json!(search_cost));
    let total = cost["total_cost_usd"].as_f64().unwrap_or(0.0) + search_cost;
    cost.insert("total_cost_usd".into(), json!(total));

    let mut out = Map::new();
    out.insert("lyrics".into(), json!(result.lyrics));
    out.insert("source_url".into(), json!(result.source_url));
    out.insert("search_queries".into(), json!(found.queries));
    out.insert("cost".into(), Value::Object(cost));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lyrics_cleanup_matches_python() {
        assert_eq!(clean_lyrics("שורה ראשונה  \r\nשורה שניה\n\n\n\nבית שני\n"), "שורה ראשונה\nשורה שניה\n\nבית שני");
    }
}
