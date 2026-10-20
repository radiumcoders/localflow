"""Voice every line of the video with Gradium TTS.

Reads scripts/lines.json ({id: {voice, text, config?}}), writes public/audio/<id>.wav (48 kHz,
loudness-normalised) and src/data/timings.json ({id: {duration, words: [{text, start, end}]}}).
Skips lines that haven't changed since the last run; name ids to redo just those.

    GRADIUM_API_KEY=gd_... python3 scripts/tts.py [ID...]
"""
import base64, hashlib, json, os, pathlib, subprocess, sys, urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
KEY = os.environ.get("GRADIUM_API_KEY") or sys.exit("set GRADIUM_API_KEY")

lines = json.loads((ROOT / "scripts/lines.json").read_text())
out_dir = ROOT / "public/audio"
data_path = ROOT / "src/data/timings.json"
timings = json.loads(data_path.read_text()) if data_path.exists() else {}
only = set(sys.argv[1:])

for lid, line in lines.items():
    if only and lid not in only:
        continue
    digest = hashlib.sha1(json.dumps(line, sort_keys=True).encode()).hexdigest()
    wav = out_dir / f"{lid}.wav"
    if not only and timings.get(lid, {}).get("hash") == digest and wav.exists():
        continue
    body = {"text": line["text"], "voice_id": line["voice"], "output_format": "wav", "only_audio": False}
    if "config" in line:
        body["json_config"] = line["config"]
    req = urllib.request.Request("https://api.gradium.ai/api/post/speech/tts", data=json.dumps(body).encode(),
                                 headers={"x-api-key": KEY, "Content-Type": "application/json"})
    audio, words = b"", []
    with urllib.request.urlopen(req) as resp:
        for raw in resp:
            if not raw.strip():
                continue
            msg = json.loads(raw)
            if msg.get("type") == "audio":
                audio += base64.b64decode(msg["audio"])
            elif msg.get("type") == "text":
                words.append({"text": msg["text"], "start": msg["start_s"], "end": msg.get("stop_s", msg["start_s"])})
            elif msg.get("type") == "error":
                sys.exit(f"{lid}: {msg}")
    tmp = out_dir / f"{lid}.raw.wav"
    tmp.write_bytes(audio)
    subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-i", str(tmp), "-af", "loudnorm=I=-16:TP=-1.5",
                    "-ar", "48000", "-ac", "1", str(wav)], check=True)
    tmp.unlink()
    dur = float(subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", str(wav)],
                               capture_output=True, text=True).stdout)
    timings[lid] = {"hash": digest, "duration": round(dur, 3), "words": words}
    print(f"{lid}: {dur:.2f}s, {len(words)} words")

data_path.write_text(json.dumps(timings, indent=1))
