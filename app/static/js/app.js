// Sing-Along Studio - Frontend Logic

// State
let allSongs = [];
let activeTab = "library";
let currentEditingSong = null;
let currentPerformance = {
  id: null,
  title: "ערב שירה בציבור",
  date: new Date().toISOString().split("T")[0],
  song_ids: []
};

// Studio State
let studioVerses = [];
let nextUntimedVerseIndex = 0;
let isPreviewMode = false;
let selectedAudioFile = null;

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

// ==================== INITIALIZATION ====================
document.addEventListener("DOMContentLoaded", () => {
  setupNavigation();
  setupLibrary();
  setupStudio();
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
      window.location.href = data.download_url;
      showToast("המצגת נוצרה והורדה בהצלחה!", "success");
    }
  } catch (err) {
    showToast("שגיאה ביצירת המצגת", "error");
  }
}

async function exportSongVideo(songId) {
  showToast("מרנדר סרטון קריוקי (עשוי לקחת מספר שניות)...", "info");
  try {
    const res = await fetch("/api/export/video", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ song_id: songId })
    });
    const data = await res.json();
    if (data.download_url) {
      window.location.href = data.download_url;
      showToast("סרטון הקריוקי נוצר בהצלחה!", "success");
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

  // Lyrics Parser buttons
  document.getElementById("btn-parse-stanzas").addEventListener("click", () => parseLyrics(true));
  document.getElementById("btn-parse-lines").addEventListener("click", () => parseLyrics(false));
  rawLyrics.addEventListener("input", () => autoDetectLanguageAndCount());

  // Audio file input
  document.getElementById("song-audio-file").addEventListener("change", (e) => {
    const file = e.target.files[0];
    if (file) {
      selectedAudioFile = file;
      audio.src = URL.createObjectURL(file);
      audio.load();
      showToast(`קובץ שמע נטען: ${file.name}`, "info");
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
      const saved = await saveCurrentSong(false);
      if (saved && saved.id) {
        exportSongVideo(saved.id);
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

function parseLyrics(byStanzas = true) {
  const raw = document.getElementById("song-raw-lyrics").value.trim();
  if (!raw) {
    showToast("נא להדביק מילים בתיבת הטקסט", "error");
    return;
  }

  studioVerses = [];
  if (byStanzas) {
    // Split by 2 or more newlines
    const blocks = raw.split(/\n\s*\n+/);
    blocks.forEach((block, idx) => {
      const clean = block.trim();
      if (clean) {
        studioVerses.push({
          id: idx + 1,
          text: clean,
          start_time: null,
          end_time: null
        });
      }
    });
  } else {
    // Split by line
    const lines = raw.split("\n");
    lines.forEach((line, idx) => {
      const clean = line.trim();
      if (clean) {
        studioVerses.push({
          id: idx + 1,
          text: clean,
          start_time: null,
          end_time: null
        });
      }
    });
  }

  nextUntimedVerseIndex = 0;
  document.getElementById("verse-count-badge").innerText = `זוהו ${studioVerses.length} בתים`;
  renderVersesList();
  updateLivePreview(0);
  showToast(`חולק בהצלחה ל-${studioVerses.length} בתים! כעת הפעל את הנגן והקש רווח לתזמון.`, "success");
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
        הדבק מילים משמאל ולחץ "פצל לפי בתים" כדי להתחיל בתזמון.
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
  document.getElementById("song-title").value = song.title || "";
  document.getElementById("song-artist").value = song.artist || "";
  document.getElementById("song-language").value = song.language || "he";

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
  studioVerses = [];
  nextUntimedVerseIndex = 0;
  document.getElementById("song-title").value = "";
  document.getElementById("song-artist").value = "";
  document.getElementById("song-raw-lyrics").value = "";
  document.getElementById("song-audio-file").value = "";
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

function loadPerformanceIntoState(perf) {
  currentPerformance = {
    id: perf.id,
    title: perf.title || "הופעה",
    date: perf.date || new Date().toISOString().split("T")[0],
    song_ids: Array.isArray(perf.song_ids) ? [...perf.song_ids] : []
  };
  document.getElementById("perf-title").value = currentPerformance.title;
  document.getElementById("perf-date").value = currentPerformance.date;
  renderSetlistQueue();
  renderSetlistAvailableSongs();
}

function resetPerformance() {
  currentPerformance = {
    id: null,
    title: "הופעה חדשה",
    date: new Date().toISOString().split("T")[0],
    song_ids: []
  };
  document.getElementById("perf-title").value = currentPerformance.title;
  document.getElementById("perf-date").value = currentPerformance.date;
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
    song_ids: currentPerformance.song_ids || []
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
      window.location.href = data.download_url;
      showToast(`המצגת נוצרה בהצלחה (${data.song_count} שירים)!`, "success");
    }
  } catch (err) {
    showToast("שגיאה ביצירת המצגת להופעה", "error");
  }
}

async function exportPerformanceVideo() {
  await savePerformance();
  showToast("מרנדר סרטון וידאו מלא לכל שירי ההופעה (MP4)... עשוי לקחת מספר רגעים", "info");
  try {
    const res = await fetch("/api/export/video", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ performance_id: currentPerformance.id })
    });
    const data = await res.json();
    if (data.download_url) {
      window.location.href = data.download_url;
      showToast(`סרטון הווידאו של ההופעה נוצר בהצלחה! (${data.song_count} שירים)`, "success");
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
      document.getElementById("fs-lyrics-text").style.color = "var(--accent-gold)";
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
  lyricsEl.style.fontSize = "3.8rem";
  lyricsEl.style.color = "var(--accent-gold)";
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

  lyricsEl.innerText = verse.text;
  lyricsEl.style.fontSize = "3.2rem";
  lyricsEl.style.color = "#ffffff";
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
