// Sing-Along Studio - Frontend Logic

// Default show colors (keep in sync with DEFAULT_THEME in app/theme.py)
const DEFAULT_PERF_COLORS = {
  bg_color: "#0b1120",
  text_color: "#ffffff",
  title_color: "#38bdf8"
};
const PERF_COLOR_INPUTS = {
  bg_color: "perf-bg-color",
  text_color: "perf-text-color",
  title_color: "perf-title-color"
};

// State
let allSongs = [];
let activeTab = "library";
let currentEditingSong = null;
let currentPerformance = {
  id: null,
  title: "ערב שירה בציבור",
  date: new Date().toISOString().split("T")[0],
  song_ids: [],
  ...DEFAULT_PERF_COLORS
};

// Studio State
let studioVerses = [];
let nextUntimedVerseIndex = 0;
let isPreviewMode = false;
let selectedAudioFile = null;
let studioYoutubeUrl = null;  // set when the song's audio came from a YouTube link (enables AI timing)

// Fullscreen Stage State
let stageSlides = [];
let currentStageSlideIndex = 0;

// Helper: Toast Notifications
function showToast(message, type = "info") {
  const container = document.getElementById("toast-container");
  const toast = document.createElement("div");
  toast.className = `toast toast-${type}`;
  toast.innerText = message;
  container.appendChild(toast);
  setTimeout(() => {
    toast.style.opacity = "0";
    setTimeout(() => toast.remove(), 300);
  }, 3500);
}

// Helper: Format Seconds to MM:SS.S
function formatTime(seconds) {
  if (isNaN(seconds) || seconds < 0) return "00:00.0";
  const mins = Math.floor(seconds / 60);
  const secs = (seconds % 60).toFixed(1);
  return `${mins.toString().padStart(2, "0")}:${secs.padStart(4, "0")}`;
}

// Helper: Detect Hebrew
function isHebrew(text) {
  return /[\u0590-\u05FF]/.test(text);
}

// Only one player at a time: the studio, library preview and live stage each have their own
// <audio>, and starting one must silence the rest (otherwise two songs play mixed together).
function setupSingleAudioPlayback() {
  // "play" doesn't bubble, so listen in the capture phase
  document.addEventListener("play", (e) => {
    document.querySelectorAll("audio").forEach(other => {
      if (other !== e.target && !other.paused) other.pause();
    });
  }, true);
}

// ==================== INITIALIZATION ====================
document.addEventListener("DOMContentLoaded", () => {
  setupSingleAudioPlayback();
  setupNavigation();
  setupLibrary();
  setupStudio();
  setupAiTiming();
  setupAiLyrics();
  setupSetlist();
  setupFullscreenStage();
  loadSongs();
  loadPerformances();
});

// ==================== NAVIGATION ====================
function setupNavigation() {
  const navBtns = document.querySelectorAll(".nav-btn");
  navBtns.forEach(btn => {
    btn.addEventListener("click", () => {
      const tab = btn.dataset.tab;
      switchTab(tab);
    });
  });

  document.getElementById("btn-add-new-song").addEventListener("click", () => {
    resetStudio();
    switchTab("studio");
  });
}

function switchTab(tabId) {
  activeTab = tabId;
  document.querySelectorAll(".nav-btn").forEach(b => {
    b.classList.toggle("active", b.dataset.tab === tabId);
  });
  document.querySelectorAll(".tab-pane").forEach(pane => {
    pane.classList.toggle("active", pane.id === `tab-${tabId}`);
  });

  if (tabId === "library") {
    loadSongs();
  } else if (tabId === "setlist") {
    renderSetlistAvailableSongs();
  }
}

// ==================== SONG LIBRARY ====================
async function loadSongs() {
  try {
    const search = document.getElementById("library-search").value;
    const lang = document.getElementById("library-lang-filter").value;
    const res = await fetch(`/api/songs?search=${encodeURIComponent(search)}&language=${lang}`);
    allSongs = await res.json();
    renderSongCards(allSongs);
    renderSetlistAvailableSongs();
  } catch (err) {
    showToast("שגיאה בטעינת ספריית השירים", "error");
    console.error(err);
  }
}

function setupLibrary() {
  document.getElementById("library-search").addEventListener("input", () => loadSongs());
  document.getElementById("library-lang-filter").addEventListener("change", () => loadSongs());

  // Modal Close
  document.getElementById("modal-close-btn").addEventListener("click", () => {
    const modal = document.getElementById("preview-modal");
    modal.classList.remove("active");
    const audio = document.getElementById("modal-audio");
    audio.pause();
    audio.src = "";
  });
}

function renderSongCards(songs) {
  const container = document.getElementById("songs-container");
  container.innerHTML = "";

  if (songs.length === 0) {
    container.innerHTML = `
      <div style="grid-column: 1 / -1; text-align: center; padding: 3rem; color: var(--text-secondary);">
        <h3>לא נמצאו שירים</h3>
        <p style="margin-top: 0.5rem;">לחץ על <strong>"הוסף שיר חדש"</strong> כדי להתחיל להזין שירים למאגר.</p>
      </div>
    `;
    return;
  }

  songs.forEach(song => {
    const card = document.createElement("div");
    card.className = "card";
    const isHe = song.language === "he";
    const verseCount = song.verses ? song.verses.length : 0;
    const durationStr = formatTime(song.duration || 0).split(".")[0];

    card.innerHTML = `
      <div class="card-header">
        <div>
          <div class="card-title">${song.title}</div>
          <div class="card-artist">${song.artist || "אמן לא צוין"}</div>
        </div>
        <span class="badge ${isHe ? 'badge-he' : 'badge-en'}">
          ${isHe ? '🇮🇱 עברית' : '🇬🇧 English'}
        </span>
      </div>

      <div class="card-meta">
        <span>📖 ${verseCount} בתים</span>
        <span>⏱️ ${durationStr}</span>
        <span>${song.has_audio ? '🎵 שמע קיים' : '🔇 ללא שמע'}</span>
      </div>

      <div class="card-actions">
        <button class="btn btn-secondary btn-sm btn-preview" title="נגן ותצוגה מקדימה">
          <span>▶️</span> נגן
        </button>
        <button class="btn btn-gold btn-sm btn-pptx" title="הורד מצגת PowerPoint">
          <span>📊</span> מצגת
        </button>
        <button class="btn btn-primary btn-sm btn-video" title="צור סרטון קריוקי MP4">
          <span>🎬</span> וידאו
        </button>
        <button class="btn btn-secondary btn-sm btn-edit" title="ערוך באולפן">
          <span>✏️</span> ערוך
        </button>
        <button class="btn btn-danger btn-sm btn-delete" title="מחק מהמאגר">
          <span>🗑️</span>
        </button>
      </div>
    `;

    // Handlers
    card.querySelector(".btn-preview").addEventListener("click", () => openPreviewModal(song));
    card.querySelector(".btn-pptx").addEventListener("click", () => exportSongPptx(song.id));
    card.querySelector(".btn-video").addEventListener("click", () => exportSongVideo(song.id));
    card.querySelector(".btn-edit").addEventListener("click", () => loadSongIntoStudio(song));
    card.querySelector(".btn-delete").addEventListener("click", () => deleteSong(song.id, song.title));

    container.appendChild(card);
  });
}

// A finished export is a file on the server. In a browser it downloads; in the desktop app (window.pywebview)
// a native Save As dialog copies it instead, because the embedded browser does not download files.
async function deliverDownload(data, successMessage) {
  const api = window.pywebview && window.pywebview.api;
  if (!api) {
    window.location.href = data.download_url;
    showToast(successMessage, "success");
    return;
  }
  const parts = data.download_url.split("/");
  const saved = await api.save_export(parts[parts.length - 2], decodeURIComponent(parts[parts.length - 1]));
  if (saved) showToast(`${successMessage} נשמר ב: ${saved}`, "success");
}

async function exportSongPptx(songId) {
  showToast("מייצר מצגת PowerPoint...", "info");
  try {
    const res = await fetch("/api/export/pptx", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ song_id: songId })
    });
    const data = await res.json();
    if (data.download_url) {
      await deliverDownload(data, "המצגת נוצרה והורדה בהצלחה!");
    }
  } catch (err) {
    showToast("שגיאה ביצירת המצגת", "error");
  }
}

// ==================== VIDEO EXPORT OPTIONS ====================
const VIDEO_OPTIONS_KEY = "videoExportOptions";
const VIDEO_FPS_DEFAULT = 5, VIDEO_FPS_MIN = 1, VIDEO_FPS_MAX = 30;

// Ask how the video should be built. Resolves to { frame_rate } (null = variable frame rate, otherwise a
// constant rate in frames per second), or to null if the user cancels. The last choice is remembered.
function askVideoOptions() {
  const modal = document.getElementById("video-options-modal");
  const fpsInput = document.getElementById("video-fps");
  const radios = Array.from(document.querySelectorAll('input[name="video-frame-mode"]'));
  const selectedMode = () => radios.find(r => r.checked).value;

  let saved = {};
  try { saved = JSON.parse(localStorage.getItem(VIDEO_OPTIONS_KEY) || "{}"); } catch (e) { /* ignore a corrupt value */ }
  const savedMode = saved.mode === "variable" ? "variable" : "constant";  // constant plays everywhere, so it is the default
  radios.forEach(r => { r.checked = r.value === savedMode; });
  fpsInput.value = saved.fps >= VIDEO_FPS_MIN && saved.fps <= VIDEO_FPS_MAX ? saved.fps : VIDEO_FPS_DEFAULT;
  const syncFpsEnabled = () => { fpsInput.disabled = selectedMode() !== "constant"; };
  syncFpsEnabled();

  return new Promise(resolve => {
    const finish = (value) => {
      modal.classList.remove("active");
      modal.onclick = null;
      document.removeEventListener("keydown", onKey);
      radios.forEach(r => { r.onchange = null; });
      resolve(value);
    };
    const submit = () => {
      const mode = selectedMode();
      const fps = Number(fpsInput.value);
      const fpsValid = fpsInput.value !== "" && fps >= VIDEO_FPS_MIN && fps <= VIDEO_FPS_MAX;
      if (mode === "constant" && !fpsValid) {
        showToast(`קצב הפריימים חייב להיות מספר בין ${VIDEO_FPS_MIN} ל-${VIDEO_FPS_MAX}`, "error");
        fpsInput.focus();
        return;
      }
      localStorage.setItem(VIDEO_OPTIONS_KEY, JSON.stringify({ mode, fps: fpsValid ? fps : VIDEO_FPS_DEFAULT }));
      finish({ frame_rate: mode === "constant" ? fps : null });
    };
    const onKey = (e) => {
      if (e.key === "Escape") finish(null);
      else if (e.key === "Enter") { e.preventDefault(); submit(); }
    };
    radios.forEach(r => { r.onchange = syncFpsEnabled; });
    document.addEventListener("keydown", onKey);
    modal.onclick = (e) => { if (e.target === modal) finish(null); };  // a click on the dark backdrop cancels
    document.getElementById("video-options-confirm").onclick = submit;
    document.getElementById("video-options-cancel").onclick = () => finish(null);
    modal.classList.add("active");
    (fpsInput.disabled ? radios.find(r => r.checked) : fpsInput).focus();
  });
}

// options: what askVideoOptions resolved to; asked for here when the caller has not already done so
async function exportSongVideo(songId, options) {
  if (options === undefined) options = await askVideoOptions();
  if (!options) return;
  showToast("מרנדר סרטון קריוקי (עשוי לקחת מספר שניות)...", "info");
  try {
    const res = await fetch("/api/export/video", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ song_id: songId, frame_rate: options.frame_rate })
    });
    const data = await res.json();
    if (data.download_url) {
      await deliverDownload(data, "סרטון הקריוקי נוצר בהצלחה!");
    } else {
      showToast("שגיאה ביצירת הסרטון: " + (data.detail || "שגיאה"), "error");
    }
  } catch (err) {
    showToast("שגיאה ברינדור הווידאו", "error");
  }
}

async function deleteSong(songId, songTitle) {
  if (!confirm(`האם אתה בטוח שברצונך למחוק את השיר "${songTitle}"?`)) return;
  try {
    await fetch(`/api/songs/${songId}`, { method: "DELETE" });
    showToast("השיר נמחק בהצלחה", "success");
    loadSongs();
  } catch (err) {
    showToast("שגיאה במחיקת השיר", "error");
  }
}

function openPreviewModal(song) {
  const modal = document.getElementById("preview-modal");
  document.getElementById("modal-song-title").innerText = song.title;
  document.getElementById("modal-song-artist").innerText = song.artist || "";
  const audio = document.getElementById("modal-audio");
  const lyricsBox = document.getElementById("modal-preview-lyrics");
  lyricsBox.innerText = song.verses && song.verses.length > 0 ? song.verses[0].text : "(אין מילים שמורות)";

  audio.src = `/api/songs/${song.id}/audio`;
  audio.ontimeupdate = () => {
    if (!song.verses) return;
    const curTime = audio.currentTime;
    const currentVerse = song.verses.find((v, idx) => {
      const nextV = song.verses[idx + 1];
      const end = nextV ? nextV.start_time : (v.end_time || curTime + 5);
      return curTime >= v.start_time && curTime <= end;
    });
    if (currentVerse) {
      lyricsBox.innerText = currentVerse.text;
    }
  };

  document.getElementById("modal-export-pptx").onclick = () => exportSongPptx(song.id);
  document.getElementById("modal-export-video").onclick = () => exportSongVideo(song.id);

  modal.classList.add("active");
  audio.play().catch(() => {});
}

// ==================== TAP-TO-SYNC STUDIO ====================
function setupStudio() {
  const audio = document.getElementById("studio-audio");
  const scrubber = document.getElementById("audio-scrubber");
  const playBtn = document.getElementById("btn-play-pause");
  const tapBtn = document.getElementById("btn-tap-sync");
  const rawLyrics = document.getElementById("song-raw-lyrics");

  // Verses are split automatically from the lyrics box as it is edited
  rawLyrics.addEventListener("input", () => {
    autoDetectLanguageAndCount();
    scheduleVerseSync();
  });

  // Audio file input
  document.getElementById("song-audio-file").addEventListener("change", (e) => {
    const file = e.target.files[0];
    if (file) {
      selectedAudioFile = file;
      studioYoutubeUrl = null;
      audio.src = URL.createObjectURL(file);
      audio.load();
      showToast(`קובץ שמע נטען: ${file.name}`, "info");
    }
  });

  // Audio from a YouTube (or other video) link
  const urlInput = document.getElementById("song-audio-url");
  const urlBtn = document.getElementById("btn-download-audio-url");
  const downloadFromUrl = async () => {
    const url = urlInput.value.trim();
    if (!url) {
      showToast("נא להדביק קישור", "error");
      return;
    }
    urlBtn.disabled = true;
    urlBtn.innerText = "⏳ מוריד ומשנה לפורמט MP3...";
    try {
      const res = await fetch("/api/audio/from-url", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ url })
      });
      if (!res.ok) {
        const err = await res.json().catch(() => ({}));
        throw new Error(err.detail || "ההורדה נכשלה");
      }
      const title = decodeURIComponent(res.headers.get("X-Audio-Title") || "");
      const artist = decodeURIComponent(res.headers.get("X-Audio-Artist") || "");
      const blob = await res.blob();
      const file = new File([blob], `${title || "audio"}.mp3`, { type: "audio/mpeg" });

      selectedAudioFile = file;
      studioYoutubeUrl = url;
      document.getElementById("song-audio-file").value = "";
      audio.src = URL.createObjectURL(file);
      audio.load();
      // Fill in the details only if the user hasn't typed their own
      const titleInput = document.getElementById("song-title");
      const artistInput = document.getElementById("song-artist");
      if (title && !titleInput.value.trim()) titleInput.value = title;
      if (artist && !artistInput.value.trim()) artistInput.value = artist;
      updateAiLyricsButton();  // setting a value from code fires no input event
      showToast(`השמע הורד והומר בהצלחה${title ? `: ${title}` : ""}`, "success");
    } catch (err) {
      showToast(err.message, "error");
    } finally {
      urlBtn.disabled = false;
      urlBtn.innerText = "⬇️ הורד והמר ל-MP3";
    }
  };
  urlBtn.addEventListener("click", downloadFromUrl);
  urlInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      downloadFromUrl();
    }
  });

  // Play/Pause
  playBtn.addEventListener("click", () => {
    if (audio.paused) {
      audio.play();
    } else {
      audio.pause();
    }
  });

  document.getElementById("btn-stop").addEventListener("click", () => {
    audio.pause();
    audio.currentTime = 0;
  });

  // Tap-to-Sync: Big Button Click
  tapBtn.addEventListener("click", () => recordTapTimestamp());

  // Tap-to-Sync: Spacebar listener
  window.addEventListener("keydown", (e) => {
    if (activeTab !== "studio") return;
    // Don't trigger if user is typing in an input or textarea
    if (e.target.tagName === "INPUT" || e.target.tagName === "TEXTAREA") return;

    if (e.code === "Space") {
      e.preventDefault();
      recordTapTimestamp();
    }
  });

  // Audio Events
  audio.addEventListener("play", () => {
    document.getElementById("play-icon").innerText = "⏸️";
    document.getElementById("play-text").innerText = "השהה";
    tapBtn.classList.add("pulsing");
  });

  audio.addEventListener("pause", () => {
    document.getElementById("play-icon").innerText = "▶️";
    document.getElementById("play-text").innerText = "נגן מוזיקה";
    tapBtn.classList.remove("pulsing");
  });

  audio.addEventListener("timeupdate", () => {
    const cur = audio.currentTime;
    const dur = audio.duration || 0;
    document.getElementById("audio-time-display").innerText = `${formatTime(cur)} / ${formatTime(dur)}`;
    if (dur > 0) {
      scrubber.value = (cur / dur) * 100;
    }
    updateLivePreview(cur);
  });

  audio.addEventListener("loadedmetadata", () => {
    document.getElementById("audio-time-display").innerText = `00:00.0 / ${formatTime(audio.duration)}`;
  });

  scrubber.addEventListener("input", () => {
    if (audio.duration) {
      audio.currentTime = (scrubber.value / 100) * audio.duration;
    }
  });

  document.getElementById("btn-clear-timings").addEventListener("click", () => {
    studioVerses.forEach(v => {
      v.start_time = null;
      v.end_time = null;
    });
    nextUntimedVerseIndex = 0;
    renderVersesList();
    updateLivePreview(audio.currentTime);
    showToast("התזמונים אופסו", "info");
  });

  document.getElementById("btn-save-song").addEventListener("click", () => saveCurrentSong(true));
  document.getElementById("btn-studio-reset").addEventListener("click", () => resetStudio());
  const studioVidBtn = document.getElementById("btn-studio-export-video");
  if (studioVidBtn) {
    studioVidBtn.addEventListener("click", async () => {
      const options = await askVideoOptions();
      if (!options) return;
      const saved = await saveCurrentSong(false);
      if (saved && saved.id) {
        exportSongVideo(saved.id, options);
      }
    });
  }
}

function autoDetectLanguageAndCount() {
  const text = document.getElementById("song-raw-lyrics").value;
  const langSelect = document.getElementById("song-language");
  if (isHebrew(text)) {
    langSelect.value = "he";
  }
}

// ---------- Automatic verse splitting ----------
// The lyrics box is the source of truth: verses are its blank-line-separated paragraphs. On every
// edit the new verses are lined up with the previous ones, so a verse that was only slightly
// edited, moved, or sits next to an edit keeps its timing. Only genuinely new verses are untimed.
const VERSE_MATCH_THRESHOLD = 0.4;  // min. text similarity (0-1) to count as "the same verse, edited"
const VERSE_SYNC_DELAY_MS = 150;
let verseSyncTimer = null;
// Recently removed timed verses; restored if their text comes back (undo, or cut and paste elsewhere)
let removedTimedVerses = [];

function isVerseTimed(verse) {
  return verse.start_time !== null && verse.start_time !== undefined && verse.start_time >= 0;
}

function firstUntimedVerseIndex() {
  const idx = studioVerses.findIndex(v => !isVerseTimed(v));
  return idx === -1 ? studioVerses.length : idx;
}

function normalizeVerseText(text) {
  return text.split("\n").map(l => l.trim()).filter(Boolean).join("\n");
}

function splitLyricsIntoVerses(raw) {
  return raw.split(/\n\s*\n+/).map(normalizeVerseText).filter(Boolean);
}

function characterBigrams(text) {
  const counts = new Map();
  const flat = text.replace(/\s+/g, " ");
  for (let i = 0; i < flat.length - 1; i++) {
    const gram = flat.slice(i, i + 2);
    counts.set(gram, (counts.get(gram) || 0) + 1);
  }
  return { text, counts, total: Math.max(0, flat.length - 1) };
}

// Dice coefficient over character pairs: 1 = identical, 0 = nothing in common
function textSimilarity(a, b) {
  if (a.text === b.text) return 1;
  if (a.total === 0 || b.total === 0) return 0;
  let overlap = 0;
  a.counts.forEach((n, gram) => { overlap += Math.min(n, b.counts.get(gram) || 0); });
  return (2 * overlap) / (a.total + b.total);
}

// Order-preserving alignment of the old verses to the new verse texts that maximizes total
// similarity. Returns, for each new verse, the index of its old verse (or -1 if it is new).
function alignVerses(oldVerses, newTexts) {
  const a = oldVerses.map(v => characterBigrams(normalizeVerseText(v.text)));
  const b = newTexts.map(characterBigrams);
  const n = a.length, m = b.length;
  const sim = (i, j) => {
    const s = textSimilarity(a[i], b[j]);
    return s >= VERSE_MATCH_THRESHOLD ? s : 0;
  };

  const score = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
  for (let i = 1; i <= n; i++) {
    for (let j = 1; j <= m; j++) {
      const s = sim(i - 1, j - 1);
      score[i][j] = Math.max(score[i - 1][j], score[i][j - 1], s > 0 ? score[i - 1][j - 1] + s : 0);
    }
  }

  const match = new Array(m).fill(-1);
  let i = n, j = m;
  while (i > 0 && j > 0) {
    const s = sim(i - 1, j - 1);
    if (s > 0 && Math.abs(score[i][j] - (score[i - 1][j - 1] + s)) < 1e-9) {
      match[j - 1] = i - 1;
      i--;
      j--;
    } else if (score[i][j] === score[i - 1][j]) {
      i--;
    } else {
      j--;
    }
  }
  return match;
}

function syncVersesFromLyrics() {
  clearTimeout(verseSyncTimer);
  const newTexts = splitLyricsIntoVerses(document.getElementById("song-raw-lyrics").value);
  const oldVerses = studioVerses;
  const oldTexts = oldVerses.map(v => v.text);
  const match = alignVerses(oldVerses, newTexts);
  const oldNext = new Map(oldVerses.map((v, i) => [v, oldVerses[i + 1]]));

  // Timed verses that no longer appear go to the "recently removed" list
  const keptOld = new Set(match.filter(i => i >= 0));
  oldVerses.forEach((v, i) => {
    if (!keptOld.has(i) && isVerseTimed(v)) removedTimedVerses.push(v);
  });
  removedTimedVerses = removedTimedVerses.slice(-100);

  let lastId = oldVerses.reduce((max, v) => Math.max(max, Number(v.id) || 0), 0);
  const verses = newTexts.map((text, j) => {
    if (match[j] >= 0) {
      const verse = oldVerses[match[j]];
      verse.text = text;
      return verse;
    }
    const removedIdx = removedTimedVerses.findIndex(r => normalizeVerseText(r.text) === text);
    if (removedIdx >= 0) {
      const [restored] = removedTimedVerses.splice(removedIdx, 1);
      return { ...restored, id: ++lastId, text };
    }
    return { id: ++lastId, text, start_time: null, end_time: null };
  });

  const unchanged = verses.length === oldVerses.length &&
    verses.every((v, k) => v === oldVerses[k] && v.text === oldTexts[k]);
  if (unchanged) return;

  // A verse whose next verse changed no longer ends where it used to: let it end when the next starts
  verses.forEach((v, k) => {
    if (oldNext.has(v) && oldNext.get(v) !== verses[k + 1]) v.end_time = null;
  });

  studioVerses = verses;
  nextUntimedVerseIndex = firstUntimedVerseIndex();
  document.getElementById("verse-count-badge").innerText = `זוהו ${verses.length} בתים`;
  renderVersesList();
  updateLivePreview(document.getElementById("studio-audio").currentTime);
}

function scheduleVerseSync() {
  clearTimeout(verseSyncTimer);
  verseSyncTimer = setTimeout(syncVersesFromLyrics, VERSE_SYNC_DELAY_MS);
}

// ---------- AI automatic timing (Gemini) ----------
const AI_MODALITY_LABELS = { video: "וידאו", audio: "שמע", text: "טקסט", image: "תמונה" };
let aiTimerInterval = null;

function escapeHtml(text) {
  return String(text).replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

function formatUsd(amount) {
  return `$${amount.toFixed(amount < 0.1 ? 4 : 3)}`;
}

function formatIsoDateHe(iso) {
  const [y, m, d] = iso.split("-");
  return `${Number(d)}.${Number(m)}.${y}`;
}

const AI_PROGRESS_YOUTUBE = "מוריד את הסרטון, מעלה אותו ל-Google ומבקש מ-Gemini לתזמן את הבתים...";
const AI_PROGRESS_SINGER = "מוריד את גרסת הזמר, מעלה את שני קבצי השמע ל-Google ומבקש מ-Gemini לתזמן את גרסת הקריוקי...";

// The song's own audio (an imported file/video, or the audio of a saved song)
function studioHasAudio() {
  return !!selectedAudioFile || !!(currentEditingSong && currentEditingSong.has_audio);
}

function setupAiTiming() {
  const btn = document.getElementById("btn-ai-timing");
  const menu = document.getElementById("ai-timing-menu");
  const youtubeOption = document.getElementById("ai-opt-youtube");
  const singerOption = document.getElementById("ai-opt-singer");
  const singerUrlInput = document.getElementById("ai-singer-url");
  const modal = document.getElementById("ai-timing-modal");

  const closeMenu = () => {
    menu.hidden = true;
    btn.setAttribute("aria-expanded", "false");
  };
  const openMenu = () => {
    // "By YouTube video" needs the song's audio to have come from a YouTube link
    const youtubeAvailable = !!studioYoutubeUrl;
    youtubeOption.disabled = !youtubeAvailable;
    document.getElementById("ai-opt-youtube-hint").innerText = youtubeAvailable ? "" : "זמין רק לשירים שיובאו מקישור יוטיוב";
    // "By singer version" needs any audio for the song: that is the karaoke version
    const hasAudio = studioHasAudio();
    singerOption.disabled = !hasAudio;
    document.getElementById("ai-opt-singer-hint").innerText = hasAudio
      ? "לשירים בלי סרטון קריוקי: צרף קישור לגרסה עם זמר"
      : "דורש קובץ שמע לשיר (גרסת הקריוקי)";
    menu.hidden = false;
    btn.setAttribute("aria-expanded", "true");
  };

  btn.addEventListener("click", (e) => {
    e.stopPropagation();
    if (menu.hidden) openMenu(); else closeMenu();
  });
  document.addEventListener("click", (e) => {
    if (!menu.hidden && !menu.contains(e.target)) closeMenu();
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") closeMenu();
  });
  youtubeOption.addEventListener("click", () => {
    closeMenu();
    timeLyricsFromYoutube();
  });
  singerOption.addEventListener("click", () => {
    closeMenu();
    showAiSingerInput();
  });

  const startSingerTiming = () => {
    const url = singerUrlInput.value.trim();
    if (!url) {
      showToast("נא להדביק קישור ליוטיוב של גרסה עם זמר", "error");
      return;
    }
    timeLyricsWithSingerVersion(url);
  };
  document.getElementById("ai-singer-start").addEventListener("click", startSingerTiming);
  singerUrlInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      startSingerTiming();
    }
  });
  document.getElementById("ai-singer-cancel").addEventListener("click", () => modal.classList.remove("active"));
  document.getElementById("ai-modal-close").addEventListener("click", () => modal.classList.remove("active"));
}

// The shared AI dialog's heading and the "usually takes" line, which differ per operation
const AI_DIALOG_TIMING = { title: "תזמון אוטומטי", usual: "זה לוקח בדרך כלל חצי דקה עד שתיים." };
const AI_DIALOG_LYRICS = { title: "חיפוש מילים", usual: "זה לוקח בדרך כלל כמה שניות." };

function setAiDialogText({ title, usual }) {
  document.getElementById("ai-modal-title").innerText = title;
  document.getElementById("ai-modal-usual").innerText = usual;
}

function showAiSingerInput() {
  setAiDialogText(AI_DIALOG_TIMING);
  document.getElementById("ai-modal-progress").hidden = true;
  document.getElementById("ai-modal-result").hidden = true;
  document.getElementById("ai-modal-close").hidden = true;
  document.getElementById("ai-modal-input").hidden = false;
  document.getElementById("ai-timing-modal").classList.add("active");
  document.getElementById("ai-singer-url").focus();
}

function showAiProgress(message) {
  document.getElementById("ai-modal-input").hidden = true;
  document.getElementById("ai-modal-progress-text").innerText = message;
  document.getElementById("ai-modal-progress").hidden = false;
  document.getElementById("ai-modal-result").hidden = true;
  document.getElementById("ai-modal-close").hidden = true;
  const elapsed = document.getElementById("ai-modal-elapsed");
  const started = Date.now();
  elapsed.innerText = "0";
  clearInterval(aiTimerInterval);
  aiTimerInterval = setInterval(() => {
    elapsed.innerText = Math.floor((Date.now() - started) / 1000);
  }, 1000);
  document.getElementById("ai-timing-modal").classList.add("active");
}

function showAiResult(html) {
  clearInterval(aiTimerInterval);
  document.getElementById("ai-modal-input").hidden = true;
  document.getElementById("ai-modal-progress").hidden = true;
  const result = document.getElementById("ai-modal-result");
  result.innerHTML = html;
  result.hidden = false;
  document.getElementById("ai-modal-close").hidden = false;
}

function renderAiCost(cost) {
  const modalities = Object.entries(cost.input_tokens_by_modality || {})
    .map(([name, n]) => `${AI_MODALITY_LABELS[name] || name} ${n.toLocaleString("en-US")}`)
    .join(" · ");
  const thinking = cost.thinking_tokens ? ` + ${cost.thinking_tokens.toLocaleString("en-US")} חשיבה` : "";
  const promo = cost.price_valid_through
    ? ` מחיר המבצע בתוקף עד ${formatIsoDateHe(cost.price_valid_through)}, ואז התעריף מוכפל.`
    : "";
  // Only requests with Google Search grounding carry a search part
  const search = cost.search_query_count === undefined ? "" : `
      <tr>
        <td>חיפושי Google: ${cost.search_query_count}<br>
            <small style="color: var(--text-muted);">$${cost.search_price_per_1000} לאלף חיפושים. ${cost.search_free_per_month.toLocaleString("en-US")} הראשונים בכל חודש חינם</small></td>
        <td>${formatUsd(cost.search_cost_usd)}</td>
      </tr>`;
  return `
    <div class="ai-result-cost">${formatUsd(cost.total_cost_usd)}</div>
    <table class="ai-cost-table">
      <tr>
        <td>קלט: ${cost.input_tokens.toLocaleString("en-US")} טוקנים${modalities ? ` (${modalities})` : ""}<br>
            <small style="color: var(--text-muted);">$${cost.input_price_per_million} למיליון טוקנים</small></td>
        <td>${formatUsd(cost.input_cost_usd)}</td>
      </tr>
      <tr>
        <td>פלט: ${cost.output_tokens.toLocaleString("en-US")} טוקנים${thinking}<br>
            <small style="color: var(--text-muted);">$${cost.output_price_per_million} למיליון טוקנים</small></td>
        <td>${formatUsd(cost.output_cost_usd)}</td>
      </tr>${search}
    </table>
    <p class="ai-note">
      זה המחיר לפי התעריף בתשלום של ${escapeHtml(cost.model)}.${promo}
      אם המפתח שלך בתוכנית החינמית של Google AI Studio, לא תחויב בפועל.
    </p>`;
}

// Shared by every AI timing option: checks, progress dialog, applying the result, showing the cost.
// sendRequest(verseTexts) must return the fetch Response of the option's API call.
async function runAiTiming(progressMessage, sendRequest) {
  syncVersesFromLyrics();  // apply any edit still waiting for the delay
  if (studioVerses.length === 0) {
    showToast("נא להוסיף מילים לפני התזמון האוטומטי", "error");
    return;
  }
  if (studioVerses.some(isVerseTimed) && !confirm("התזמונים הקיימים יוחלפו בתזמון האוטומטי. להמשיך?")) {
    return;
  }

  const sentTexts = studioVerses.map(v => v.text);
  setAiDialogText(AI_DIALOG_TIMING);
  showAiProgress(progressMessage);
  try {
    const res = await sendRequest(sentTexts);
    const data = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(data.detail || "התזמון האוטומטי נכשל");

    // Apply to the verses that still have the text that was sent (the lyrics may have been edited meanwhile)
    const notes = [...data.warnings];
    let timed = 0;
    data.timings.forEach((timing, i) => {
      const verse = studioVerses[i];
      if (!verse || verse.text !== sentTexts[i]) return;
      verse.start_time = timing.start_time;
      verse.end_time = timing.end_time;
      if (timing.start_time !== null) timed++;
    });
    if (studioVerses.length !== sentTexts.length || studioVerses.some((v, i) => v.text !== sentTexts[i])) {
      notes.push("המילים שונו בזמן התזמון, ולכן חלק מהבתים לא עודכנו");
    }
    nextUntimedVerseIndex = firstUntimedVerseIndex();
    renderVersesList();
    updateLivePreview(document.getElementById("studio-audio").currentTime);

    showAiResult(`
      <p style="font-weight: 700;">✅ תוזמנו ${timed} מתוך ${sentTexts.length} בתים</p>
      ${notes.length ? `<div class="ai-warnings">${notes.map(n => `<div>⚠️ ${escapeHtml(n)}</div>`).join("")}</div>` : ""}
      <p class="ai-note" style="margin-top: 0.4rem;">זו הערכה של בינה מלאכותית. מומלץ להאזין ולכוון בתים עם ‎-0.5s / +0.5s‎ לפי הצורך.</p>
      ${renderAiCost(data.cost)}
    `);
  } catch (err) {
    showAiResult(`<p style="color: var(--accent-red); font-weight: 700;">❌ ${escapeHtml(err.message)}</p>`);
  }
}

// Option 1: the song's audio came from a YouTube video that shows/sings the lyrics
function timeLyricsFromYoutube() {
  if (!studioYoutubeUrl) {
    showToast("התזמון לפי יוטיוב זמין רק לשיר שיובא מקישור יוטיוב", "error");
    return;
  }
  return runAiTiming(AI_PROGRESS_YOUTUBE, (verseTexts) => fetch("/api/ai/time-verses", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      youtube_url: studioYoutubeUrl,
      verses: verseTexts,
      title: document.getElementById("song-title").value.trim(),
      artist: document.getElementById("song-artist").value.trim()
    })
  }));
}

// Option 2: the song's audio is a karaoke version; a YouTube version with a singer guides the timing
function timeLyricsWithSingerVersion(singerUrl) {
  if (!studioHasAudio()) {
    showToast("לשיר אין קובץ שמע (גרסת קריוקי)", "error");
    return;
  }
  return runAiTiming(AI_PROGRESS_SINGER, (verseTexts) => {
    const form = new FormData();
    form.append("singer_url", singerUrl);
    form.append("verses", JSON.stringify(verseTexts));
    form.append("title", document.getElementById("song-title").value.trim());
    form.append("artist", document.getElementById("song-artist").value.trim());
    if (selectedAudioFile) {
      form.append("karaoke_audio", selectedAudioFile, selectedAudioFile.name || "karaoke.mp3");
    } else if (currentEditingSong && currentEditingSong.id) {
      form.append("song_id", currentEditingSong.id);  // the server reads the saved audio itself
    }
    return fetch("/api/ai/time-verses-singer", { method: "POST", body: form });
  });
}

// ---------- AI lyrics search (Gemini + Google Search) ----------
function updateAiLyricsButton() {
  const ready = !!(document.getElementById("song-title").value.trim() && document.getElementById("song-artist").value.trim());
  document.getElementById("btn-ai-lyrics").disabled = !ready;
  document.getElementById("ai-lyrics-hint").hidden = ready;
}

function setupAiLyrics() {
  for (const id of ["song-title", "song-artist"]) {
    document.getElementById(id).addEventListener("input", updateAiLyricsButton);
  }
  document.getElementById("btn-ai-lyrics").addEventListener("click", findLyricsWithAi);
  updateAiLyricsButton();
}

async function findLyricsWithAi() {
  const title = document.getElementById("song-title").value.trim();
  const artist = document.getElementById("song-artist").value.trim();
  if (!title || !artist) return;
  const box = document.getElementById("song-raw-lyrics");
  if (box.value.trim() && !confirm("המילים הקיימות יוחלפו במילים שנמצאו. להמשיך?")) return;

  setAiDialogText(AI_DIALOG_LYRICS);
  showAiProgress(`מחפש ב-Google את המילים של "${title}" של ${artist}...`);
  try {
    const res = await fetch("/api/ai/find-lyrics", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ title, artist })
    });
    const data = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(data.detail || "חיפוש המילים נכשל");

    box.value = data.lyrics;
    autoDetectLanguageAndCount();
    syncVersesFromLyrics();
    showAiResult(`
      <p style="font-weight: 700;">✅ נמצאו מילים (${studioVerses.length} בתים)</p>
      ${data.source_url ? `<p class="ai-note" style="margin-top: 0.4rem;">מקור: <span dir="ltr">${escapeHtml(new URL(data.source_url).hostname.replace(/^www\./, ""))}</span></p>` : ""}
      <p class="ai-note" style="margin-top: 0.4rem;">Gemini מצא את הדף בחיפוש בגוגל והעתיק ממנו את המילים. מומלץ לעבור עליהן לפני השימוש.</p>
      ${renderAiCost(data.cost)}
    `);
  } catch (err) {
    showAiResult(`<p style="color: var(--accent-red); font-weight: 700;">❌ ${escapeHtml(err.message)}</p>`);
  }
}

function recordTapTimestamp() {
  const audio = document.getElementById("studio-audio");
  if (audio.paused) {
    // If paused, start playback on first tap!
    audio.play();
  }

  if (nextUntimedVerseIndex >= studioVerses.length) {
    showToast("כל הבתים כבר תוזמנו! השיר מוכן.", "info");
    return;
  }

  const curTime = Number(audio.currentTime.toFixed(1));
  const verse = studioVerses[nextUntimedVerseIndex];
  verse.start_time = curTime;

  // Set previous verse end time
  if (nextUntimedVerseIndex > 0) {
    const prev = studioVerses[nextUntimedVerseIndex - 1];
    if (!prev.end_time || prev.end_time < prev.start_time) {
      prev.end_time = Math.max(prev.start_time + 1.0, curTime - 0.2);
    }
  }

  nextUntimedVerseIndex++;
  renderVersesList();
  updateLivePreview(curTime);

  // Scroll to active verse in list
  const activeEl = document.getElementById(`verse-row-${verse.id}`);
  if (activeEl) {
    activeEl.scrollIntoView({ behavior: "smooth", block: "nearest" });
  }
}

function renderVersesList() {
  const container = document.getElementById("verses-list");
  container.innerHTML = "";

  if (studioVerses.length === 0) {
    container.innerHTML = `
      <div style="color: var(--text-muted); text-align: center; padding: 2rem;">
        הדבק מילים משמאל והן יתפצלו לבתים אוטומטית (שורה ריקה מפרידה בין בתים), ואז התחל בתזמון.
      </div>
    `;
    return;
  }

  const audio = document.getElementById("studio-audio");

  studioVerses.forEach((verse, idx) => {
    const row = document.createElement("div");
    row.id = `verse-row-${verse.id}`;
    row.className = `verse-item ${idx < nextUntimedVerseIndex ? 'passed' : ''} ${idx === nextUntimedVerseIndex ? 'active' : ''}`;

    const isTimed = verse.start_time !== null && verse.start_time !== undefined && verse.start_time >= 0;
    const timeStr = isTimed ? formatTime(verse.start_time) : "--:--.-";

    row.innerHTML = `
      <div class="verse-meta">
        <span style="font-weight: 700;">בית ${idx + 1} מתוך ${studioVerses.length}</span>
        <div style="display: flex; gap: 0.4rem; align-items: center;">
          <span class="verse-time-badge">${timeStr}</span>
          <button class="btn btn-secondary btn-sm btn-nudge-minus" title="הקדש 0.5 שנ'">-0.5s</button>
          <button class="btn btn-secondary btn-sm btn-nudge-plus" title="אחר 0.5 שנ'">+0.5s</button>
          <button class="btn btn-secondary btn-sm btn-set-now" title="קבע לזמן הנוכחי">⏱️ כעת</button>
          <button class="btn btn-secondary btn-sm btn-seek" title="קפוץ לבית זה">▶️</button>
        </div>
      </div>
      <div class="verse-text-content" dir="${isHebrew(verse.text) ? 'rtl' : 'ltr'}">${verse.text}</div>
    `;

    // Nudge and Time Set Handlers
    row.querySelector(".btn-nudge-minus").onclick = () => {
      if (verse.start_time !== null) {
        verse.start_time = Math.max(0, Number((verse.start_time - 0.5).toFixed(1)));
        renderVersesList();
        updateLivePreview(audio.currentTime);
      }
    };
    row.querySelector(".btn-nudge-plus").onclick = () => {
      if (verse.start_time !== null) {
        verse.start_time = Number((verse.start_time + 0.5).toFixed(1));
        renderVersesList();
        updateLivePreview(audio.currentTime);
      }
    };
    row.querySelector(".btn-set-now").onclick = () => {
      verse.start_time = Number(audio.currentTime.toFixed(1));
      renderVersesList();
      updateLivePreview(audio.currentTime);
    };
    row.querySelector(".btn-seek").onclick = () => {
      if (verse.start_time !== null) {
        audio.currentTime = verse.start_time;
        audio.play();
      }
    };

    container.appendChild(row);
  });
}

function updateLivePreview(curTime) {
  const title = document.getElementById("song-title").value || "שם השיר";
  const artist = document.getElementById("song-artist").value;
  document.getElementById("stage-preview-title").innerText = artist ? `${title} • ${artist}` : title;

  const previewLyrics = document.getElementById("stage-preview-text");
  const previewNext = document.getElementById("stage-preview-next");

  if (!studioVerses || studioVerses.length === 0) {
    previewLyrics.innerText = "המילים יופיעו כאן בזמן אמת...";
    previewNext.innerText = "";
    return;
  }

  // Get all verses that currently have a valid timestamp
  const timedVerses = studioVerses
    .map((v, idx) => ({ ...v, originalIndex: idx }))
    .filter(v => v.start_time !== null && v.start_time !== undefined && v.start_time >= 0);

  // If no verses are timed yet, show the first verse as waiting to start!
  if (timedVerses.length === 0) {
    const firstV = studioVerses[0];
    previewLyrics.innerText = firstV.text;
    previewLyrics.setAttribute("dir", isHebrew(firstV.text) ? "rtl" : "ltr");
    previewNext.innerText = "הקש [רווח] כדי לסמן את תחילת הבית הראשון";
    return;
  }

  // If the audio is currently playing before the first timed verse starts
  if (curTime < timedVerses[0].start_time) {
    previewLyrics.innerText = "מוזיקת פתיחה...";
    previewLyrics.setAttribute("dir", "rtl");
    const firstLine = timedVerses[0].text.split("\n")[0];
    previewNext.innerText = `בית ראשון מתחיל ב: ${formatTime(timedVerses[0].start_time)} ("${firstLine}")`;
    return;
  }

  // Find active verse among timed verses:
  let activeVerse = null;
  let activeOriginalIndex = -1;

  for (let i = 0; i < timedVerses.length; i++) {
    const tv = timedVerses[i];
    if (curTime >= tv.start_time) {
      let endTime = null;
      if (i + 1 < timedVerses.length) {
        endTime = timedVerses[i + 1].start_time;
      } else if (tv.end_time && tv.end_time > tv.start_time) {
        endTime = tv.end_time;
      }

      if (endTime === null || curTime < endTime) {
        activeVerse = tv;
        activeOriginalIndex = tv.originalIndex;
      }
    }
  }

  if (activeVerse) {
    previewLyrics.innerText = activeVerse.text;
    previewLyrics.setAttribute("dir", isHebrew(activeVerse.text) ? "rtl" : "ltr");

    // Preview subsequent verse
    if (activeOriginalIndex + 1 < studioVerses.length) {
      const nextV = studioVerses[activeOriginalIndex + 1];
      const firstLine = nextV.text.split("\n")[0];
      const isNextTimed = nextV.start_time !== null && nextV.start_time !== undefined;
      const timeTag = isNextTimed ? `[${formatTime(nextV.start_time)}]` : "(הבא לתזמון - הקש רווח)";
      previewNext.innerText = `הבא ${timeTag}: ${firstLine}`;
    } else {
      previewNext.innerText = "סוף השיר";
    }
  } else {
    previewLyrics.innerText = "סוף השיר 🎵";
    previewNext.innerText = "";
  }
}

async function saveCurrentSong(redirect = true) {
  syncVersesFromLyrics();  // apply any edit still waiting for the delay
  const title = document.getElementById("song-title").value.trim();
  const artist = document.getElementById("song-artist").value.trim();
  const language = document.getElementById("song-language").value;
  const audio = document.getElementById("studio-audio");

  if (!title) {
    showToast("נא להזין שם שיר", "error");
    return null;
  }
  if (studioVerses.length === 0) {
    showToast("נא לפצל את המילים לבתים", "error");
    return null;
  }

  const songData = {
    id: currentEditingSong ? currentEditingSong.id : undefined,
    title,
    artist,
    language,
    youtube_url: studioYoutubeUrl || undefined,
    duration: audio.duration || (studioVerses[studioVerses.length - 1].start_time + 6.0),
    verses: studioVerses
  };

  const formData = new FormData();
  formData.append("song_json", JSON.stringify(songData));
  if (selectedAudioFile) {
    formData.append("audio_file", selectedAudioFile);
  }

  showToast("שומר שיר במאגר הקבצים...", "info");
  try {
    const res = await fetch("/api/songs", {
      method: "POST",
      body: formData
    });
    const saved = await res.json();
    currentEditingSong = saved;
    showToast(`השיר "${saved.title}" נשמר בהצלחה בספרייה!`, "success");
    if (redirect) {
      resetStudio();
      switchTab("library");
    }
    return saved;
  } catch (err) {
    showToast("שגיאה בשמירת השיר", "error");
    console.error(err);
    return null;
  }
}

function loadSongIntoStudio(song) {
  resetStudio();
  currentEditingSong = song;
  studioYoutubeUrl = song.youtube_url || null;
  document.getElementById("song-title").value = song.title || "";
  document.getElementById("song-artist").value = song.artist || "";
  document.getElementById("song-language").value = song.language || "he";
  updateAiLyricsButton();

  // Join verses into raw text
  const rawText = song.verses.map(v => v.text).join("\n\n");
  document.getElementById("song-raw-lyrics").value = rawText;

  studioVerses = JSON.parse(JSON.stringify(song.verses || []));
  nextUntimedVerseIndex = studioVerses.length;
  renderVersesList();

  const audio = document.getElementById("studio-audio");
  audio.src = `/api/songs/${song.id}/audio`;
  audio.load();

  switchTab("studio");
  showToast(`נטען השיר: ${song.title}`, "info");
}

function resetStudio() {
  currentEditingSong = null;
  selectedAudioFile = null;
  studioYoutubeUrl = null;
  studioVerses = [];
  removedTimedVerses = [];
  nextUntimedVerseIndex = 0;
  document.getElementById("song-title").value = "";
  document.getElementById("song-artist").value = "";
  updateAiLyricsButton();
  document.getElementById("song-raw-lyrics").value = "";
  document.getElementById("song-audio-file").value = "";
  document.getElementById("song-audio-url").value = "";
  document.getElementById("verse-count-badge").innerText = "זוהו 0 בתים";
  const audio = document.getElementById("studio-audio");
  audio.pause();
  audio.src = "";
  renderVersesList();
}

// ==================== SETLIST BUILDER ====================
let allPerformances = [];

function setupSetlist() {
  document.getElementById("setlist-search").addEventListener("input", () => renderSetlistAvailableSongs());
  document.getElementById("btn-save-performance").addEventListener("click", () => savePerformance());
  document.getElementById("btn-new-performance").addEventListener("click", () => {
    document.getElementById("perf-selector").value = "NEW";
    resetPerformance();
    showToast("נפתחה הופעה חדשה ונקייה. כעת הוסף שירים מהמאגר.", "info");
  });
  document.getElementById("btn-clear-setlist-queue").addEventListener("click", () => {
    currentPerformance.song_ids = [];
    renderSetlistQueue();
    renderSetlistAvailableSongs();
    showToast("רשימת השירים בהופעה רוקנה", "info");
  });
  document.getElementById("btn-delete-performance").addEventListener("click", async () => {
    if (!currentPerformance.id) {
      resetPerformance();
      return;
    }
    if (!confirm(`האם אתה בטוח שברצונך למחוק את ההופעה "${currentPerformance.title}"?`)) return;
    try {
      await fetch(`/api/performances/${currentPerformance.id}`, { method: "DELETE" });
      showToast("ההופעה נמחקה בהצלחה", "success");
      await loadPerformances("NEW");
    } catch (err) {
      showToast("שגיאה במחיקת ההופעה", "error");
    }
  });

  const selector = document.getElementById("perf-selector");
  if (selector) {
    selector.addEventListener("change", (e) => {
      const val = e.target.value;
      if (val === "NEW") {
        resetPerformance();
      } else {
        const perf = allPerformances.find(p => p.id === val);
        if (perf) {
          loadPerformanceIntoState(perf);
        }
      }
    });
  }

  Object.entries(PERF_COLOR_INPUTS).forEach(([key, inputId]) => {
    document.getElementById(inputId).addEventListener("input", (e) => {
      currentPerformance[key] = e.target.value;
    });
  });
  document.getElementById("btn-reset-perf-colors").addEventListener("click", () => {
    Object.assign(currentPerformance, DEFAULT_PERF_COLORS);
    renderPerformanceColors();
  });

  document.getElementById("btn-export-perf-pptx").addEventListener("click", () => exportPerformancePptx());
  document.getElementById("btn-export-perf-video").addEventListener("click", () => exportPerformanceVideo());
  document.getElementById("btn-launch-fullscreen").addEventListener("click", () => launchFullscreenFromSetlist());
}

async function loadPerformances(selectedId = null) {
  try {
    const res = await fetch("/api/performances");
    allPerformances = await res.json();

    const selector = document.getElementById("perf-selector");
    if (selector) {
      selector.innerHTML = "";

      const newOpt = document.createElement("option");
      newOpt.value = "NEW";
      newOpt.innerText = "➕ [צור הופעה חדשה ריקה...]";
      selector.appendChild(newOpt);

      allPerformances.forEach(p => {
        const opt = document.createElement("option");
        opt.value = p.id;
        const count = p.song_ids ? p.song_ids.length : 0;
        opt.innerText = `${p.title} (${count} שירים)`;
        selector.appendChild(opt);
      });

      if (selectedId && allPerformances.some(p => p.id === selectedId)) {
        selector.value = selectedId;
        const perf = allPerformances.find(p => p.id === selectedId);
        loadPerformanceIntoState(perf);
      } else {
        // Default to a clean new empty performance!
        selector.value = "NEW";
        resetPerformance();
      }
    }
  } catch (err) {
    console.error("Error loading performances", err);
  }
}

function performanceColors(perf) {
  const colors = {};
  Object.keys(DEFAULT_PERF_COLORS).forEach(key => {
    const value = perf && perf[key];
    colors[key] = /^#[0-9a-f]{6}$/i.test(value || "") ? value.toLowerCase() : DEFAULT_PERF_COLORS[key];
  });
  return colors;
}

function renderPerformanceColors() {
  Object.entries(PERF_COLOR_INPUTS).forEach(([key, inputId]) => {
    document.getElementById(inputId).value = currentPerformance[key];
  });
}

function loadPerformanceIntoState(perf) {
  currentPerformance = {
    id: perf.id,
    title: perf.title || "הופעה",
    date: perf.date || new Date().toISOString().split("T")[0],
    song_ids: Array.isArray(perf.song_ids) ? [...perf.song_ids] : [],
    ...performanceColors(perf)
  };
  document.getElementById("perf-title").value = currentPerformance.title;
  document.getElementById("perf-date").value = currentPerformance.date;
  renderPerformanceColors();
  renderSetlistQueue();
  renderSetlistAvailableSongs();
}

function resetPerformance() {
  currentPerformance = {
    id: null,
    title: "הופעה חדשה",
    date: new Date().toISOString().split("T")[0],
    song_ids: [],
    ...DEFAULT_PERF_COLORS
  };
  document.getElementById("perf-title").value = currentPerformance.title;
  document.getElementById("perf-date").value = currentPerformance.date;
  renderPerformanceColors();
  renderSetlistQueue();
  renderSetlistAvailableSongs();
}

function renderSetlistAvailableSongs() {
  const container = document.getElementById("setlist-available-songs");
  if (!container) return;
  const search = (document.getElementById("setlist-search")?.value || "").toLowerCase();

  const filtered = allSongs.filter(s => {
    return s.title.toLowerCase().includes(search) || (s.artist && s.artist.toLowerCase().includes(search));
  });

  container.innerHTML = "";
  filtered.forEach(song => {
    const isAdded = currentPerformance.song_ids && currentPerformance.song_ids.includes(song.id);
    const addedCount = currentPerformance.song_ids ? currentPerformance.song_ids.filter(id => id === song.id).length : 0;

    const item = document.createElement("div");
    item.className = "verse-item";
    item.style.flexDirection = "row";
    item.style.justifyContent = "space-between";
    item.style.alignItems = "center";
    if (isAdded) {
      item.style.borderColor = "var(--accent-cyan)";
      item.style.backgroundColor = "rgba(56, 189, 248, 0.05)";
    }

    item.innerHTML = `
      <div>
        <strong style="color: var(--text-primary);">${song.title}</strong>
        <div style="font-size: 0.85rem; color: var(--accent-gold);">${song.artist || ""}</div>
      </div>
      <div style="display: flex; gap: 0.5rem; align-items: center;">
        ${isAdded ? `<span style="font-size: 0.8rem; color: var(--accent-cyan); font-weight: 700;">✓ ברשימה (${addedCount})</span>` : ''}
        <button class="btn btn-secondary btn-sm btn-add-to-set">
          <span>${isAdded ? '➕ הוסף שוב' : '➕ הוסף'}</span>
        </button>
      </div>
    `;

    item.querySelector(".btn-add-to-set").onclick = () => {
      currentPerformance.song_ids.push(song.id);
      renderSetlistQueue();
      renderSetlistAvailableSongs();
      showToast(`נוסף להופעה: ${song.title}`, "info");
    };

    container.appendChild(item);
  });
}

function renderSetlistQueue() {
  const container = document.getElementById("perf-songs-list");
  container.innerHTML = "";

  if (!currentPerformance.song_ids) {
    currentPerformance.song_ids = [];
  }

  const selectedSongs = currentPerformance.song_ids.map(id => allSongs.find(s => s.id === id)).filter(Boolean);

  document.getElementById("perf-count").innerText = selectedSongs.length;
  const totalMins = Math.round(selectedSongs.reduce((sum, s) => sum + (s.duration || 0), 0) / 60);
  document.getElementById("perf-duration").innerText = `${totalMins} דק'`;

  if (selectedSongs.length === 0) {
    container.innerHTML = `
      <div style="text-align: center; color: var(--text-muted); padding: 2rem;">
        הרשימה ריקה. בחר שירים מהמאגר בצד שמאל והוסף אותם להופעה ⬅️
      </div>
    `;
    return;
  }

  selectedSongs.forEach((song, idx) => {
    const row = document.createElement("div");
    row.className = "verse-item";
    row.style.flexDirection = "row";
    row.style.justifyContent = "space-between";
    row.style.alignItems = "center";

    row.innerHTML = `
      <div style="display: flex; gap: 0.75rem; align-items: center;">
        <span style="font-size: 1.1rem; font-weight: 800; color: var(--accent-cyan); width: 24px;">${idx + 1}.</span>
        <div>
          <strong style="font-size: 1.05rem;">${song.title}</strong>
          <div style="font-size: 0.85rem; color: var(--accent-gold);">${song.artist || ""}</div>
        </div>
      </div>

      <div style="display: flex; gap: 0.35rem;">
        <button class="btn btn-secondary btn-sm btn-up" ${idx === 0 ? 'disabled' : ''} title="העבר למעלה">↑</button>
        <button class="btn btn-secondary btn-sm btn-down" ${idx === selectedSongs.length - 1 ? 'disabled' : ''} title="העבר למטה">↓</button>
        <button class="btn btn-danger btn-sm btn-remove" title="הסר מההופעה">✕</button>
      </div>
    `;

    row.querySelector(".btn-up").onclick = () => {
      if (idx > 0) {
        const temp = currentPerformance.song_ids[idx];
        currentPerformance.song_ids[idx] = currentPerformance.song_ids[idx - 1];
        currentPerformance.song_ids[idx - 1] = temp;
        renderSetlistQueue();
        renderSetlistAvailableSongs();
      }
    };

    row.querySelector(".btn-down").onclick = () => {
      if (idx < selectedSongs.length - 1) {
        const temp = currentPerformance.song_ids[idx];
        currentPerformance.song_ids[idx] = currentPerformance.song_ids[idx + 1];
        currentPerformance.song_ids[idx + 1] = temp;
        renderSetlistQueue();
        renderSetlistAvailableSongs();
      }
    };

    row.querySelector(".btn-remove").onclick = () => {
      currentPerformance.song_ids.splice(idx, 1);
      renderSetlistQueue();
      renderSetlistAvailableSongs();
    };

    container.appendChild(row);
  });
}

async function savePerformance() {
  const title = document.getElementById("perf-title").value.trim() || "ערב שירה";
  const date = document.getElementById("perf-date").value;

  const perfData = {
    id: currentPerformance.id || undefined,
    title: title,
    date: date,
    song_ids: currentPerformance.song_ids || [],
    ...performanceColors(currentPerformance)
  };

  try {
    const res = await fetch("/api/performances", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(perfData)
    });
    const saved = await res.json();
    currentPerformance = saved;
    await loadPerformances(saved.id);
    showToast(`רשימת ההופעה "${saved.title}" נשמרה בהצלחה!`, "success");
    return saved;
  } catch (err) {
    showToast("שגיאה בשמירת ההופעה", "error");
    return null;
  }
}

async function exportPerformancePptx() {
  await savePerformance();
  showToast("מייצר מצגת PowerPoint לכל השירים בהופעה...", "info");
  try {
    const res = await fetch("/api/export/pptx", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ performance_id: currentPerformance.id })
    });
    const data = await res.json();
    if (data.download_url) {
      await deliverDownload(data, `המצגת נוצרה בהצלחה (${data.song_count} שירים)!`);
    }
  } catch (err) {
    showToast("שגיאה ביצירת המצגת להופעה", "error");
  }
}

async function exportPerformanceVideo() {
  const options = await askVideoOptions();
  if (!options) return;
  await savePerformance();
  showToast("מרנדר סרטון וידאו מלא לכל שירי ההופעה (MP4)... עשוי לקחת מספר רגעים", "info");
  try {
    const res = await fetch("/api/export/video", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ performance_id: currentPerformance.id, frame_rate: options.frame_rate })
    });
    const data = await res.json();
    if (data.download_url) {
      await deliverDownload(data, `סרטון הווידאו של ההופעה נוצר בהצלחה! (${data.song_count} שירים)`);
    } else {
      showToast("שגיאה ביצירת הווידאו: " + (data.detail || "שגיאה"), "error");
    }
  } catch (err) {
    showToast("שגיאה ברינדור סרטון הווידאו של ההופעה", "error");
    console.error(err);
  }
}

// ==================== FULLSCREEN LIVE STAGE & MUSIC PLAYER ====================
let fsSongs = [];
let fsCurrentSongIndex = 0;
let fsCurrentVerseIndex = -1;
let fsAutoSync = true;
let fsAutohideTimer = null;
// Font sizes (px) for the current song on the live stage, fitted to its longest title/verse
let fsSongSizes = { header: 22, title: 60, verse: 51 };

const FS_LINE_HEIGHT = 1.4;
const FS_FIT_SAFETY = 0.95;

// Width of each line at 100px in the given element's font, via canvas (handles Hebrew shaping)
function measureLinesAt100px(el, lines) {
  const ctx = (measureLinesAt100px.canvas ||= document.createElement("canvas")).getContext("2d");
  const style = getComputedStyle(el);
  ctx.font = `${style.fontWeight} 100px ${style.fontFamily}`;
  return lines.map(line => ctx.measureText(line).width / 100);
}

// Largest px size at which every block of lines fits a boxW x boxH box
function fitFontPx(el, blocks, boxW, boxH, lineHeight, maxPx) {
  let size = maxPx;
  blocks.forEach(lines => {
    if (!lines.length) return;
    const widest = Math.max(...measureLinesAt100px(el, lines));
    if (widest > 0) size = Math.min(size, (boxW * FS_FIT_SAFETY) / widest);
    size = Math.min(size, (boxH * FS_FIT_SAFETY) / (lines.length * lineHeight));
  });
  return Math.floor(size);
}

function stageVerseLines(verse) {
  return (verse.text || "").split("\n").map(l => l.trim()).filter(Boolean);
}

function computeStageSizes(song) {
  const stage = document.getElementById("fullscreen-stage");
  const lyricsEl = document.getElementById("fs-lyrics-text");
  const headerEl = document.querySelector("#fullscreen-stage .fs-header");
  const w = stage.clientWidth || window.innerWidth;
  const h = stage.clientHeight || window.innerHeight;
  const verses = song.verses || [];

  // Header: song title + progress label on one line, up to 7% of the screen height
  const progress = `בית ${verses.length} מתוך ${verses.length}`;
  const header = fitFontPx(headerEl, [[`${song.title}      ${progress}`]], w * 0.8, h * 0.07, 1.2, h * 0.07);

  // Lyrics are centered, so keep them clear of the header on top and the footer + player dock below
  const topReserved = 32 + header * 1.2 + 24;
  const bottomReserved = 170;
  const boxW = (w - 96) * 0.9;
  const boxH = h - 2 * Math.max(topReserved, bottomReserved);

  const verse = fitFontPx(lyricsEl, verses.map(stageVerseLines), boxW, boxH, FS_LINE_HEIGHT, h * 0.25);
  const titleLines = song.artist ? [song.title, "", song.artist] : [song.title];
  const title = fitFontPx(lyricsEl, [titleLines], boxW, boxH, FS_LINE_HEIGHT, h * 0.2);
  fsSongSizes = { header, title, verse: Math.max(20, verse) };
  headerEl.style.fontSize = `${header}px`;
}

function applyStageColors() {
  const stage = document.getElementById("fullscreen-stage");
  const colors = performanceColors(currentPerformance);
  stage.style.background = colors.bg_color;
  stage.style.setProperty("--stage-text-color", colors.text_color);
  stage.style.setProperty("--stage-title-color", colors.title_color);
}

function setupFullscreenStage() {
  const stage = document.getElementById("fullscreen-stage");
  const audio = document.getElementById("fs-audio");
  const playBtn = document.getElementById("fs-play-btn");
  const prevBtn = document.getElementById("fs-prev-song");
  const nextBtn = document.getElementById("fs-next-song");
  const restartBtn = document.getElementById("fs-restart-song");
  const scrubber = document.getElementById("fs-timeline-scrubber");
  const autoSyncBtn = document.getElementById("fs-toggle-autosync");
  const dock = document.getElementById("fs-player-dock");

  document.getElementById("fs-close-btn").addEventListener("click", () => closeFullscreenStage());

  // Audio Playback Events
  playBtn.addEventListener("click", () => {
    if (audio.paused) {
      audio.play().catch(e => console.warn(e));
    } else {
      audio.pause();
    }
  });

  audio.addEventListener("play", () => {
    playBtn.innerText = "⏸️";
  });

  audio.addEventListener("pause", () => {
    playBtn.innerText = "▶️";
  });

  prevBtn.addEventListener("click", () => {
    if (fsCurrentSongIndex > 0) {
      loadLiveStageSong(fsCurrentSongIndex - 1);
    }
  });

  nextBtn.addEventListener("click", () => {
    if (fsCurrentSongIndex + 1 < fsSongs.length) {
      loadLiveStageSong(fsCurrentSongIndex + 1);
    }
  });

  restartBtn.addEventListener("click", () => {
    audio.currentTime = 0;
    audio.play().catch(() => {});
  });

  autoSyncBtn.addEventListener("click", () => {
    fsAutoSync = !fsAutoSync;
    document.getElementById("fs-autosync-dot").innerText = fsAutoSync ? "🟢" : "⚪";
    document.getElementById("fs-autosync-label").innerText = fsAutoSync ? "סנכרון מוזיקה: פועל" : "סנכרון מוזיקה: כבוי";
    showToast(fsAutoSync ? "סנכרון שקופיות אוטומטי הופעל" : "סנכרון מוזיקה כבוי (מעבר ידני)", "info");
  });

  scrubber.addEventListener("input", () => {
    if (audio.duration) {
      audio.currentTime = (scrubber.value / 100) * audio.duration;
    }
  });

  audio.addEventListener("timeupdate", () => {
    const cur = audio.currentTime;
    const dur = audio.duration || 0;
    document.getElementById("fs-cur-time").innerText = formatTime(cur).split(".")[0];
    document.getElementById("fs-dur-time").innerText = formatTime(dur).split(".")[0];
    if (dur > 0) {
      scrubber.value = (cur / dur) * 100;
    }

    if (fsAutoSync) {
      syncStageSlideToAudio(cur);
    }
  });

  audio.addEventListener("ended", () => {
    if (fsCurrentSongIndex + 1 < fsSongs.length) {
      showToast("השיר הסתיים. עובר לשיר הבא בהופעה...", "info");
      setTimeout(() => {
        loadLiveStageSong(fsCurrentSongIndex + 1);
      }, 1000);
    } else {
      document.getElementById("fs-lyrics-text").innerText = "סיום ההופעה! 🎵\nתודה רבה לכולם!";
      document.getElementById("fs-lyrics-text").style.color = "var(--stage-title-color)";
      document.getElementById("fs-next-preview").innerText = "";
    }
  });

  // Autohide dock on mouse inactivity
  stage.addEventListener("mousemove", () => {
    dock.classList.remove("autohide");
    clearTimeout(fsAutohideTimer);
    fsAutohideTimer = setTimeout(() => {
      dock.classList.add("autohide");
    }, 3500);
  });

  window.addEventListener("resize", () => {
    const song = fsSongs[fsCurrentSongIndex];
    if (!stage.classList.contains("active") || !song) return;
    computeStageSizes(song);
    const lyricsEl = document.getElementById("fs-lyrics-text");
    lyricsEl.style.fontSize = `${fsCurrentVerseIndex === -1 ? fsSongSizes.title : fsSongSizes.verse}px`;
  });

  // Keyboard navigation
  window.addEventListener("keydown", (e) => {
    if (!stage.classList.contains("active")) return;

    if (e.key === "Escape") {
      closeFullscreenStage();
    } else if (e.code === "Space") {
      e.preventDefault();
      if (audio.paused) {
        audio.play().catch(() => {});
      } else {
        audio.pause();
      }
    } else if (e.key === "ArrowRight" || e.key === "PageDown") {
      e.preventDefault();
      advanceLiveStageVerse(1);
    } else if (e.key === "ArrowLeft" || e.key === "PageUp") {
      e.preventDefault();
      advanceLiveStageVerse(-1);
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      if (fsCurrentSongIndex + 1 < fsSongs.length) loadLiveStageSong(fsCurrentSongIndex + 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (fsCurrentSongIndex > 0) loadLiveStageSong(fsCurrentSongIndex - 1);
    }
  });
}

function launchFullscreenFromSetlist() {
  fsSongs = currentPerformance.song_ids.map(id => allSongs.find(s => s.id === id)).filter(Boolean);
  if (fsSongs.length === 0) {
    showToast("נא להוסיף לפחות שיר אחד להופעה", "error");
    return;
  }

  const stage = document.getElementById("fullscreen-stage");
  applyStageColors();
  stage.classList.add("active");
  if (document.documentElement.requestFullscreen) {
    document.documentElement.requestFullscreen().catch(() => {});
  }

  loadLiveStageSong(0);
}

function loadLiveStageSong(index) {
  if (index < 0 || index >= fsSongs.length) return;
  fsCurrentSongIndex = index;
  const song = fsSongs[index];

  const audio = document.getElementById("fs-audio");
  document.getElementById("fs-dock-title").innerText = `${index + 1}. ${song.title}`;
  document.getElementById("fs-dock-artist").innerText = song.artist || "";

  // Set audio source
  audio.src = `/api/songs/${song.id}/audio`;
  audio.load();

  // Show intro slide
  computeStageSizes(song);
  showStageTitleSlide(song);

  // Auto-play audio
  audio.play().catch(e => {
    console.log("Audio autoplay waiting for user interaction:", e);
  });
}

function showStageTitleSlide(song) {
  fsCurrentVerseIndex = -1;
  const titleEl = document.getElementById("fs-song-title");
  const indexEl = document.getElementById("fs-verse-index");
  const lyricsEl = document.getElementById("fs-lyrics-text");
  const nextEl = document.getElementById("fs-next-preview");

  titleEl.innerText = song.title;
  indexEl.innerText = `שיר ${fsCurrentSongIndex + 1} מתוך ${fsSongs.length}`;
  lyricsEl.innerText = song.title + (song.artist ? `\n\n${song.artist}` : "");
  lyricsEl.style.fontSize = `${fsSongSizes.title}px`;
  lyricsEl.style.color = "var(--stage-title-color)";
  lyricsEl.setAttribute("dir", isHebrew(song.title) ? "rtl" : "ltr");

  const firstVerse = (song.verses && song.verses.length > 0) ? song.verses[0] : null;
  if (firstVerse) {
    const firstLine = firstVerse.text.split("\n")[0];
    nextEl.innerText = `הבית הראשון: ${firstLine}`;
  } else {
    nextEl.innerText = "";
  }
}

function syncStageSlideToAudio(curTime) {
  const song = fsSongs[fsCurrentSongIndex];
  if (!song || !song.verses || song.verses.length === 0) return;

  // Find matching verse
  const verseIdx = song.verses.findIndex((v, idx) => {
    const nextV = song.verses[idx + 1];
    const end = nextV ? nextV.start_time : (v.end_time || v.start_time + 6.0);
    return curTime >= v.start_time && curTime <= end;
  });

  if (verseIdx !== -1 && verseIdx !== fsCurrentVerseIndex) {
    displayStageVerse(verseIdx);
  } else if (verseIdx === -1 && curTime < (song.verses[0]?.start_time || 0)) {
    if (fsCurrentVerseIndex !== -1) {
      showStageTitleSlide(song);
    }
  }
}

function displayStageVerse(verseIdx) {
  const song = fsSongs[fsCurrentSongIndex];
  if (!song || !song.verses || !song.verses[verseIdx]) return;

  fsCurrentVerseIndex = verseIdx;
  const verse = song.verses[verseIdx];

  const titleEl = document.getElementById("fs-song-title");
  const indexEl = document.getElementById("fs-verse-index");
  const lyricsEl = document.getElementById("fs-lyrics-text");
  const nextEl = document.getElementById("fs-next-preview");

  titleEl.innerText = song.title;
  indexEl.innerText = `בית ${verseIdx + 1} מתוך ${song.verses.length}`;

  lyricsEl.innerText = stageVerseLines(verse).join("\n");
  lyricsEl.style.fontSize = `${fsSongSizes.verse}px`;
  lyricsEl.style.color = "var(--stage-text-color)";
  lyricsEl.setAttribute("dir", isHebrew(verse.text) ? "rtl" : "ltr");

  if (verseIdx + 1 < song.verses.length) {
    const nextV = song.verses[verseIdx + 1];
    const firstLine = nextV.text.split("\n")[0];
    nextEl.innerText = `(הבא): ${firstLine}`;
  } else {
    if (fsCurrentSongIndex + 1 < fsSongs.length) {
      nextEl.innerText = `(השיר הבא): ${fsSongs[fsCurrentSongIndex + 1].title}`;
    } else {
      nextEl.innerText = "סוף השיר";
    }
  }
}

function advanceLiveStageVerse(direction) {
  const song = fsSongs[fsCurrentSongIndex];
  if (!song || !song.verses || song.verses.length === 0) return;

  const nextIdx = fsCurrentVerseIndex + direction;
  const audio = document.getElementById("fs-audio");

  if (nextIdx >= 0 && nextIdx < song.verses.length) {
    // Jump audio to verse start time and display verse
    audio.currentTime = song.verses[nextIdx].start_time;
    displayStageVerse(nextIdx);
  } else if (nextIdx < 0) {
    audio.currentTime = 0;
    showStageTitleSlide(song);
  } else if (nextIdx >= song.verses.length && direction > 0) {
    if (fsCurrentSongIndex + 1 < fsSongs.length) {
      loadLiveStageSong(fsCurrentSongIndex + 1);
    }
  }
}

function closeFullscreenStage() {
  const stage = document.getElementById("fullscreen-stage");
  stage.classList.remove("active");
  const audio = document.getElementById("fs-audio");
  audio.pause();
  audio.src = "";
  if (document.exitFullscreen) {
    document.exitFullscreen().catch(() => {});
  }
}
