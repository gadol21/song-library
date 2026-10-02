import os
import math
import struct
import subprocess
import tempfile
from pathlib import Path
from app.database import list_songs, save_song, save_performance, list_performances

def create_synth_mp3(chord_progressions, duration=24.0) -> bytes:
    """Generate a clean, pleasant harmonic acoustic backing track for demos."""
    sample_rate = 44100
    num_samples = int(sample_rate * duration)
    samples = []
    
    chord_len = duration / len(chord_progressions)
    
    for i in range(num_samples):
        t = i / sample_rate
        c_idx = min(int(t / chord_len), len(chord_progressions) - 1)
        chord = chord_progressions[c_idx]
        
        # gentle rhythmic pulse
        pulse = math.exp(-6.0 * (t % 1.0)) * 0.12
        
        val = 0.0
        for f in chord:
            val += 0.22 * math.sin(2 * math.pi * f * t)
            val += 0.06 * math.sin(4 * math.pi * f * t)
        val += pulse
        
        # fade in/out
        env = min(1.0, t * 1.5) * min(1.0, (duration - t) * 1.5)
        val *= env
        
        ival = int(max(-32767, min(32767, val * 32767)))
        samples.append(ival)
        
    with tempfile.NamedTemporaryFile(suffix='.wav', delete=False) as wav_file:
        wav_name = wav_file.name
        with wave.open(wav_name, 'w') as wf:
            wf.setnchannels(1)
            wf.setsampwidth(2)
            wf.setframerate(sample_rate)
            wf.writeframes(struct.pack(f'<{len(samples)}h', *samples))
            
    with tempfile.NamedTemporaryFile(suffix='.mp3', delete=False) as mp3_file:
        mp3_name = mp3_file.name
        
    subprocess.run(['ffmpeg', '-y', '-i', wav_name, '-b:a', '128k', mp3_name], check=True, capture_output=True)
    with open(mp3_name, 'rb') as f:
        mp3_bytes = f.read()
        
    os.unlink(wav_name)
    os.unlink(mp3_name)
    return mp3_bytes

import wave

def seed_sample_data_if_empty():
    """Seed sample Hebrew and English songs if no songs exist."""
    existing_songs = list_songs()
    if existing_songs:
        return

    print("Seeding sample songs for Sing-Along Studio...")

    # Song 1: Hebrew Classic
    hebrew_chords = [
        [261.63, 329.63, 392.00], # C
        [220.00, 261.63, 329.63], # Am
        [174.61, 220.00, 261.63], # F
        [196.00, 246.94, 293.66], # G
    ]
    hebrew_mp3 = create_synth_mp3(hebrew_chords, duration=22.0)
    
    song1 = {
        "id": "od-lo-tamu",
        "title": "עוד לא תמו כל פלאייך",
        "artist": "רמי קלינשטיין / נעמי שמר",
        "language": "he",
        "duration": 22.0,
        "tags": ["שירה בציבור", "ישראלי", "קלאסיקה"],
        "verses": [
            {
                "id": 1,
                "start_time": 1.5,
                "end_time": 7.5,
                "text": "ארצנו הקטנטונת, ארצנו היפה\nמולדת בלי כותונת, מולדת יחפה"
            },
            {
                "id": 2,
                "start_time": 8.0,
                "end_time": 14.5,
                "text": "קבליני אל שירייך, כלה יפהפיה\nפתחי לי שערייך, אבוא בם אודה יה"
            },
            {
                "id": 3,
                "start_time": 15.0,
                "end_time": 21.5,
                "text": "עוד לא תמו כל פלאייך\nעוד נושק אותו הים\nעוד נמשך השיר עליך\nשיר יפה עד בלי סוף ועד עולם"
            }
        ]
    }
    save_song(song1, audio_bytes=hebrew_mp3, audio_ext=".mp3")

    # Song 2: English Classic
    english_chords = [
        [220.00, 261.63, 329.63], # Am
        [174.61, 220.00, 261.63], # F
        [196.00, 246.94, 293.66], # G
        [261.63, 329.63, 392.00], # C
    ]
    english_mp3 = create_synth_mp3(english_chords, duration=20.0)

    song2 = {
        "id": "stand-by-me",
        "title": "Stand By Me",
        "artist": "Ben E. King",
        "language": "en",
        "duration": 20.0,
        "tags": ["Classics", "Sing-along", "Soul"],
        "verses": [
            {
                "id": 1,
                "start_time": 1.5,
                "end_time": 6.8,
                "text": "When the night has come\nAnd the land is dark\nAnd the moon is the only light we'll see"
            },
            {
                "id": 2,
                "start_time": 7.2,
                "end_time": 12.8,
                "text": "No I won't be afraid\nOh, I won't be afraid\nJust as long as you stand, stand by me"
            },
            {
                "id": 3,
                "start_time": 13.2,
                "end_time": 19.5,
                "text": "So darling, darling, stand by me\nOh, stand by me\nOh, stand, stand by me, stand by me"
            }
        ]
    }
    save_song(song2, audio_bytes=english_mp3, audio_ext=".mp3")

    # Seed Sample Performance
    perf = {
        "id": "opening-gala",
        "title": "ערב שירה בציבור - מופע פתיחה",
        "subtitle": "מבחר שירים ישראליים ובינלאומיים",
        "date": "2026-10-15",
        "song_ids": ["od-lo-tamu", "stand-by-me"]
    }
    save_performance(perf)
    print("Sample data seeded successfully!")
