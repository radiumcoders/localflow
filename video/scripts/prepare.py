"""Per-word start times and the pill's microphone levels for each voiced line.

Gradium sends some words in chunks ("Um, so, can we move the, uh, design"): their words are
spread evenly up to the next chunk. Levels are RMS per video frame (30 fps), scaled the way
localflow's meter is, peak about 1. Writes "timed" into src/data/timings.json and
src/data/levels.json.
"""
import array, json, math, pathlib, subprocess

ROOT = pathlib.Path(__file__).resolve().parent.parent
FPS = 30
data = ROOT / "src/data/timings.json"
timings = json.loads(data.read_text())
levels = {}
for lid, t in timings.items():
    chunks = t["words"]
    timed = []
    for i, c in enumerate(chunks):
        ws = c["text"].split()
        end = chunks[i + 1]["start"] if i + 1 < len(chunks) else max(c["end"], c["start"] + 0.3 * len(ws))
        step = (end - c["start"]) / max(len(ws), 1)
        timed += [{"text": w, "start": round(c["start"] + k * step, 3)} for k, w in enumerate(ws)]
    t["timed"] = timed
    pcm = subprocess.run(["ffmpeg", "-loglevel", "error", "-i", str(ROOT / f"public/audio/{lid}.wav"),
                          "-f", "s16le", "-ac", "1", "-ar", "48000", "-"], capture_output=True, check=True).stdout
    s = array.array("h", pcm)
    n = 48000 // FPS
    rms = [math.sqrt(sum(x * x for x in s[i:i + n]) / max(len(s[i:i + n]), 1)) / 32768 for i in range(0, len(s), n)]
    peak = max(rms) or 1
    levels[lid] = [round(min(1.0, (r / peak) ** 0.7), 3) for r in rms]
data.write_text(json.dumps(timings, indent=1))
(ROOT / "src/data/levels.json").write_text(json.dumps(levels))
print({k: len(v) for k, v in levels.items()})
