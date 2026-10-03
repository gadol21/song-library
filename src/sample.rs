//! Two sample songs and a setlist, created when the library is empty (the Python app's sample_data.py).

use serde_json::{json, Value};

use crate::audio::{self, Pcm};
use crate::log;
use crate::storage;

/// A clean, pleasant harmonic backing track for demos (mono, 44.1 kHz), as the Python app synthesized it.
fn synth_mp3(chords: &[[f64; 3]], duration: f64) -> Vec<u8> {
    let rate = 44100u32;
    let n = (rate as f64 * duration) as usize;
    let chord_len = duration / chords.len() as f64;
    let mut samples = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f64 / rate as f64;
        let c = ((t / chord_len) as usize).min(chords.len() - 1);
        // gentle rhythmic pulse
        let pulse = (-6.0 * (t % 1.0)).exp() * 0.12;
        let mut val = 0.0;
        for f in chords[c] {
            val += 0.22 * (2.0 * std::f64::consts::PI * f * t).sin();
            val += 0.06 * (4.0 * std::f64::consts::PI * f * t).sin();
        }
        val += pulse;
        // fade in/out
        let env = (t * 1.5).min(1.0) * ((duration - t) * 1.5).min(1.0);
        val *= env;
        let ival = (val * 32767.0).clamp(-32767.0, 32767.0).trunc();
        samples.push((ival / 32768.0) as f32);
    }
    audio::encode_mp3(&Pcm { rate, channels: 1, samples }, 128).unwrap_or_default()
}

fn dict(v: Value) -> crate::py::Dict {
    match v {
        Value::Object(m) => m,
        _ => unreachable!(),
    }
}

pub fn seed_sample_data_if_empty() {
    if !storage::list_songs(None, None).is_empty() {
        return;
    }
    log::info("Seeding sample songs for Sing-Along Studio...");

    // Song 1: Hebrew classic
    let hebrew = [[261.63, 329.63, 392.00], [220.00, 261.63, 329.63], [174.61, 220.00, 261.63], [196.00, 246.94, 293.66]];
    let song1 = dict(json!({
        "id": "od-lo-tamu",
        "title": "עוד לא תמו כל פלאייך",
        "artist": "רמי קלינשטיין / נעמי שמר",
        "language": "he",
        "duration": 22.0,
        "tags": ["שירה בציבור", "ישראלי", "קלאסיקה"],
        "verses": [
            {"id": 1, "start_time": 1.5, "end_time": 7.5, "text": "ארצנו הקטנטונת, ארצנו היפה\nמולדת בלי כותונת, מולדת יחפה"},
            {"id": 2, "start_time": 8.0, "end_time": 14.5, "text": "קבליני אל שירייך, כלה יפהפיה\nפתחי לי שערייך, אבוא בם אודה יה"},
            {"id": 3, "start_time": 15.0, "end_time": 21.5, "text": "עוד לא תמו כל פלאייך\nעוד נושק אותו הים\nעוד נמשך השיר עליך\nשיר יפה עד בלי סוף ועד עולם"},
        ],
    }));
    let _ = storage::save_song(song1, Some((&synth_mp3(&hebrew, 22.0), ".mp3")));

    // Song 2: English classic
    let english = [[220.00, 261.63, 329.63], [174.61, 220.00, 261.63], [196.00, 246.94, 293.66], [261.63, 329.63, 392.00]];
    let song2 = dict(json!({
        "id": "stand-by-me",
        "title": "Stand By Me",
        "artist": "Ben E. King",
        "language": "en",
        "duration": 20.0,
        "tags": ["Classics", "Sing-along", "Soul"],
        "verses": [
            {"id": 1, "start_time": 1.5, "end_time": 6.8, "text": "When the night has come\nAnd the land is dark\nAnd the moon is the only light we'll see"},
            {"id": 2, "start_time": 7.2, "end_time": 12.8, "text": "No I won't be afraid\nOh, I won't be afraid\nJust as long as you stand, stand by me"},
            {"id": 3, "start_time": 13.2, "end_time": 19.5, "text": "So darling, darling, stand by me\nOh, stand by me\nOh, stand, stand by me, stand by me"},
        ],
    }));
    let _ = storage::save_song(song2, Some((&synth_mp3(&english, 20.0), ".mp3")));

    let perf = dict(json!({
        "id": "opening-gala",
        "title": "ערב שירה בציבור - מופע פתיחה",
        "subtitle": "מבחר שירים ישראליים ובינלאומיים",
        "date": "2026-10-15",
        "song_ids": ["od-lo-tamu", "stand-by-me"],
    }));
    let _ = storage::save_performance(perf);
    log::info("Sample data seeded successfully!");
}
