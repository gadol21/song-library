"""Find a song's lyrics: Gemini searches Google, the app fetches the pages it found, and Gemini copies the lyrics out.

Why not simply ask Gemini (with Google Search) for the lyrics: its copyright filter cut most such answers off
(finish reason RECITATION, measured at about 4 in 5 on a 1968 Hebrew song), and neither a unique prompt nor a higher
temperature (Google's troubleshooting advice) helped. Copying lyrics out of a page that is given to it is not
blocked (20 of 20) and gives the same text every time, so the steps are:
  1. Gemini, with Google Search, is asked which websites have the lyrics (a question it doesn't answer by reciting).
     The pages it found come back as links in the answer's grounding metadata.
  2. The app downloads those pages and turns them into plain text (no site-specific parsing).
  3. Gemini, with no tools, is given the pages and returns the lyrics from the best one.
"""
import json
import logging
import re
from concurrent.futures import ThreadPoolExecutor
from html.parser import HTMLParser
from typing import Any, Dict, List, Optional
from urllib.parse import urlparse

import httpx
from google import genai
from google.genai import errors as genai_errors
from google.genai import types

from app.gemini_timing import (
    GeminiTimingError, _api_key, _friendly_api_error, _generate_content, compute_cost, thinking_level,
)

# uvicorn's logger, so the lines show in the console / the desktop app's log. Logging (unlike print) never fails the
# request when the console can't show Hebrew.
log = logging.getLogger("uvicorn.error")

# Grounding with Google Search is billed per search query Gemini runs, on top of the tokens: the first 5,000 queries
# a month (shared by all Gemini 3.x models) are free, then $14 per 1,000. https://ai.google.dev/gemini-api/docs/pricing
SEARCH_PRICE_PER_1000 = 14.0
SEARCH_FREE_PER_MONTH = 5000

# How many of the found pages are downloaded and shown to Gemini, and how much of each
MAX_PAGES = 4
MAX_PAGE_CHARS = 20_000
FETCH_TIMEOUT_SECONDS = 15
# Video sites come up in the search but their pages have no lyrics text
SKIPPED_HOSTS = ("youtube.com", "youtu.be")
# Some lyrics sites refuse requests that don't look like a browser
FETCH_HEADERS = {
    "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Safari/537.36",
    "Accept-Language": "he,en;q=0.8",
}

# ---------------------------------------------------------------- step 1: search

def build_search_prompt(title: str, artist: str) -> str:
    # Asking for the pages' exact titles makes Gemini actually search: asked for the websites' names, it sometimes
    # (4 in 36) listed the usual lyrics sites from memory, which gives no links. Measured 72 of 72 with links.
    return (f'Search Google for web pages that show the full lyrics of the song "{title}" by {artist}. '
            "Reply only with the exact titles of the pages in the search results.")

def _grounding(response: Any) -> Any:
    candidates = getattr(response, "candidates", None) or []
    return getattr(candidates[0], "grounding_metadata", None) if candidates else None

def search_pages(client: genai.Client, title: str, artist: str) -> Dict[str, Any]:
    """Step 1. Returns {"links": [(site, url)...] in Gemini's order, "queries": [...], "cost": {...}}."""
    config = types.GenerateContentConfig(
        tools=[types.Tool(google_search=types.GoogleSearch())],
        thinking_config=types.ThinkingConfig(thinking_level=thinking_level()),
        automatic_function_calling=types.AutomaticFunctionCallingConfig(disable=True),
    )
    response = _generate_content(client, [build_search_prompt(title, artist)], config)
    metadata = _grounding(response)
    links = []
    for chunk in getattr(metadata, "grounding_chunks", None) or []:
        web = getattr(chunk, "web", None)
        if web and web.uri:
            links.append((web.title or "", web.uri))
    return {
        "links": links,
        "queries": list(getattr(metadata, "web_search_queries", None) or []),
        "cost": compute_cost(response.usage_metadata),
    }

# ---------------------------------------------------------------- step 2: fetch

class _PageText(HTMLParser):
    """The visible text of an HTML page, with line breaks where the page has them."""
    HIDDEN = {"script", "style", "noscript", "svg", "head", "template"}
    BREAKS = {"br", "p", "div", "li", "tr", "td", "h1", "h2", "h3", "h4", "h5", "h6", "section", "article", "pre"}

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.parts: List[str] = []
        self.hidden = 0

    def handle_starttag(self, tag, attrs):
        if tag in self.HIDDEN:
            self.hidden += 1
        elif tag in self.BREAKS:
            self.parts.append("\n")

    def handle_endtag(self, tag):
        if tag in self.HIDDEN:
            self.hidden = max(0, self.hidden - 1)
        elif tag in self.BREAKS:
            self.parts.append("\n")

    def handle_data(self, data):
        if not self.hidden:
            self.parts.append(data)

def html_to_text(html: str) -> str:
    parser = _PageText()
    parser.feed(html)
    text = "".join(parser.parts)
    text = re.sub(r"[ \t\r\f\v\xa0]+", " ", text)
    text = re.sub(r" *\n *", "\n", text)
    return re.sub(r"\n{3,}", "\n\n", text).strip()

def _decode(response: httpx.Response) -> str:
    """Older Hebrew sites declare windows-1255 only inside the page (<meta charset>), not in the HTTP header."""
    declared = re.search(r"""charset=["']?([\w-]+)""", response.content[:4096].decode("ascii", "ignore"), re.I)
    for encoding in (declared and declared.group(1), response.charset_encoding, "utf-8"):
        if encoding:
            try:
                return response.content.decode(encoding, errors="replace")
            except LookupError:
                continue
    return response.content.decode("utf-8", errors="replace")

def fetch_page(url: str) -> Optional[Dict[str, str]]:
    """Download a page (following Google's redirect link) as plain text; None if it can't be read."""
    try:
        response = httpx.get(url, follow_redirects=True, timeout=FETCH_TIMEOUT_SECONDS, headers=FETCH_HEADERS)
    except httpx.HTTPError:
        return None
    host = urlparse(str(response.url)).hostname or ""
    if response.status_code != 200 or any(host == h or host.endswith("." + h) for h in SKIPPED_HOSTS):
        return None
    if "html" not in response.headers.get("content-type", "html"):
        return None
    text = html_to_text(_decode(response))
    return {"url": str(response.url), "text": text[:MAX_PAGE_CHARS]} if text else None

def fetch_pages(links: List[Any]) -> List[Dict[str, str]]:
    """Step 2. The first MAX_PAGES pages that could be read, in the search's order."""
    candidates = [url for site, url in links if not any(site == h or site.endswith("." + h) for h in SKIPPED_HOSTS)]
    with ThreadPoolExecutor(max_workers=8) as pool:
        pages = list(pool.map(fetch_page, candidates[:MAX_PAGES * 2]))
    return [page for page in pages if page][:MAX_PAGES]

# ---------------------------------------------------------------- step 3: extract

_EXTRACT_SCHEMA = {
    "type": "object",
    "properties": {
        "page": {"type": "integer", "description": "Number of the page the lyrics were taken from, or 0 if no page has them"},
        "lyrics": {"type": "string", "description": "The lyrics, or an empty string"},
    },
    "required": ["page", "lyrics"],
}

def build_extract_prompt(title: str, artist: str, pages: List[Dict[str, str]]) -> str:
    listing = "\n\n".join(f'<page number="{i}" url="{p["url"]}">\n{p["text"]}\n</page>' for i, p in enumerate(pages, 1))
    return f"""Below are web pages found for the song "{title}" by {artist}. Pick the page with the most complete and \
reliable lyrics of this song and copy its lyrics exactly as written there.

The lyrics must contain no additional text:
- no title, artist, credits, introduction, explanation or notes
- no section labels such as [Chorus] or "Verse 1", and no chords
- a single blank line between verses (stanzas); within a verse, one line per sung line

If none of the pages has the lyrics of this song, answer with page 0 and empty lyrics.

{listing}"""

def clean_lyrics(raw: str) -> str:
    """Tidy line endings and blank lines."""
    lines = [line.rstrip() for line in (raw or "").replace("\r\n", "\n").split("\n")]
    return re.sub(r"\n{3,}", "\n\n", "\n".join(lines)).strip()

def extract_lyrics(client: genai.Client, title: str, artist: str, pages: List[Dict[str, str]]) -> Dict[str, Any]:
    """Step 3. Returns {"lyrics", "source_url", "cost"}; lyrics is "" when no page has them."""
    config = types.GenerateContentConfig(
        response_mime_type="application/json",
        response_json_schema=_EXTRACT_SCHEMA,
        thinking_config=types.ThinkingConfig(thinking_level=thinking_level()),
        automatic_function_calling=types.AutomaticFunctionCallingConfig(disable=True),
    )
    response = _generate_content(client, [build_extract_prompt(title, artist, pages)], config)
    cost = compute_cost(response.usage_metadata)
    if not response.text:
        candidates = getattr(response, "candidates", None) or []
        reason = getattr(candidates[0], "finish_reason", None) if candidates else None
        log.warning("find_lyrics: extraction for %r by %r gave no answer (finish reason %s)", title, artist, reason)
        raise GeminiTimingError(f"Gemini לא החזיר את המילים (סיבה: {getattr(reason, 'name', reason)}). אפשר להדביק אותן ידנית.")
    try:
        answer = json.loads(response.text)
        page_number, lyrics = int(answer.get("page") or 0), clean_lyrics(answer.get("lyrics") or "")
    except (ValueError, TypeError, AttributeError):
        raise GeminiTimingError("התשובה של Gemini לא הייתה בפורמט הצפוי. נסה שוב.")
    source = pages[page_number - 1]["url"] if 1 <= page_number <= len(pages) else None
    return {"lyrics": lyrics if source else "", "source_url": source, "cost": cost}

# ---------------------------------------------------------------- together

def _add_costs(total: Dict[str, Any], cost: Dict[str, Any]) -> Dict[str, Any]:
    """Sum the costs of two requests (the prices are the same in both)."""
    for key in ("input_tokens", "output_tokens", "thinking_tokens", "input_cost_usd", "output_cost_usd", "total_cost_usd"):
        total[key] += cost[key]
    for name, count in cost["input_tokens_by_modality"].items():
        total["input_tokens_by_modality"][name] = total["input_tokens_by_modality"].get(name, 0) + count
    return total

def find_lyrics(title: str, artist: str) -> Dict[str, Any]:
    """Find a song's lyrics on the web. Returns {"lyrics", "source_url", "search_queries", "cost"}."""
    title, artist = title.strip(), artist.strip()
    if not title or not artist:
        raise GeminiTimingError("יש למלא גם שם שיר וגם שם אמן כדי לחפש מילים.")
    key = _api_key()
    client = genai.Client(api_key=key)
    try:
        found = search_pages(client, title, artist)
        pages = fetch_pages(found["links"])
        log.info("find_lyrics: %r by %r: search found %d links, read %d pages: %s",
                 title, artist, len(found["links"]), len(pages), [p["url"] for p in pages])
        if not pages:
            raise GeminiTimingError("לא נמצאו דפי מילים לשיר הזה. בדוק את שם השיר והאמן, או הדבק את המילים ידנית.")
        result = extract_lyrics(client, title, artist, pages)
    except genai_errors.APIError as e:
        raise _friendly_api_error(e, key)
    if not result["lyrics"]:
        raise GeminiTimingError("לא נמצאו מילים לשיר הזה בדפים שנמצאו. בדוק את שם השיר והאמן, או הדבק את המילים ידנית.")

    cost = _add_costs(found["cost"], result["cost"])
    # List price: whether the searches are actually charged depends on the month's free allowance, which we can't see
    cost["search_query_count"] = len(found["queries"])
    cost["search_price_per_1000"] = SEARCH_PRICE_PER_1000
    cost["search_free_per_month"] = SEARCH_FREE_PER_MONTH
    cost["search_cost_usd"] = len(found["queries"]) * SEARCH_PRICE_PER_1000 / 1000
    cost["total_cost_usd"] += cost["search_cost_usd"]
    return {"lyrics": result["lyrics"], "source_url": result["source_url"], "search_queries": found["queries"], "cost": cost}
