"""Build the packaged app: PyInstaller, then the installer (.msi on Windows,
.dmg on macOS).

    pip install pyinstaller==6.19.0 pillow==11.3.0
    dotnet tool install --global wix --version 5.0.2    # Windows, for the .msi
    python pkg/build.py --version 0.1.0

Writes the app to build/dist/ and the installer to build/:
localflow-<version>-x64.msi, or localflow-<version>-<arm64|x64>.dmg. The
scripts have no .py extension, so they are copied in as modules for
PyInstaller to find their imports.
"""

import argparse
import os
import platform
import plistlib
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PKG = ROOT / "pkg"
BUILD = ROOT / "build"
STAGE = BUILD / "stage"


def icon():
    """A violet rounded square with the dictation pill's level bars."""
    from PIL import Image, ImageDraw
    n = 1024
    img = Image.new("RGBA", (n, n))
    top, bottom = (91, 76, 230), (40, 26, 120)
    grad = Image.new("RGBA", (1, n))
    for y in range(n):
        t = y / (n - 1)
        grad.putpixel((0, y), tuple(round(a + (b - a) * t) for a, b in zip(top, bottom)) + (255,))
    mask = Image.new("L", (n, n))
    ImageDraw.Draw(mask).rounded_rectangle((40, 40, n - 40, n - 40), radius=220, fill=255)
    img.paste(grad.resize((n, n)), (0, 0), mask)
    d = ImageDraw.Draw(img)
    d.rounded_rectangle((170, 362, n - 170, n - 362), radius=150, fill=(255, 255, 255, 255))
    heights = [70, 150, 220, 150, 70]
    for i, h in enumerate(heights):
        x = n // 2 + (i - 2) * 92
        d.rounded_rectangle((x - 24, n // 2 - h // 2, x + 24, n // 2 + h // 2), radius=24, fill=(74, 58, 200, 255))
    png, ico = BUILD / "icon.png", BUILD / "icon.ico"
    img.resize((256, 256), Image.LANCZOS).save(png)
    img.save(ico, sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    return png, ico


def stage():
    shutil.rmtree(STAGE, ignore_errors=True)
    STAGE.mkdir(parents=True)
    for src, dest in [("dictate", "dictate.py"), ("localflow", "localflow.py"), ("install.py", "install.py")]:
        shutil.copy(ROOT / src, STAGE / dest)
    for name in ("main.py", "setup_app.py"):
        shutil.copy(PKG / name, STAGE / name)


def pyinstaller(png, ico, version):
    mac = platform.system() == "Darwin"
    data = [ROOT / "words.txt", ROOT / "cases.txt", png] + ([ROOT / "paste"] if platform.system() == "Linux" else [])
    cmd = [sys.executable, "-m", "PyInstaller", "--noconfirm", "--clean", "--windowed", "--onedir",
           "--name", "localflow", "--icon", str(png if mac else ico), "--paths", str(STAGE),
           "--distpath", str(BUILD / "dist"), "--workpath", str(BUILD / "work"), "--specpath", str(BUILD)]
    for f in data:
        cmd += ["--add-data", f"{f}{os.pathsep}."]
    if mac:
        cmd += ["--osx-bundle-identifier", "computer.localflow"]
    subprocess.run(cmd + [str(STAGE / "main.py")], check=True)
    if not mac:
        return BUILD / "dist/localflow"
    app = BUILD / "dist/localflow.app"
    info = app / "Contents/Info.plist"
    with open(info, "rb") as f:
        plist = plistlib.load(f)
    plist.update(CFBundleDisplayName="localflow", CFBundleShortVersionString=version, CFBundleVersion=version,
                 NSMicrophoneUsageDescription="localflow records what you say while you hold the dictation key, "
                                              "to turn it into text.",
                 LSMinimumSystemVersion="11.0")
    with open(info, "wb") as f:
        plistlib.dump(plist, f)
    # Changing Info.plist breaks PyInstaller's ad-hoc signature; sign again (no
    # Developer ID yet: Gatekeeper asks once, see README).
    subprocess.run(["codesign", "--force", "--deep", "--sign", "-", str(app)], check=True)
    return app


def dmg(app, version):
    arch = "arm64" if platform.machine() == "arm64" else "x64"
    out = BUILD / f"localflow-{version}-{arch}.dmg"
    folder = BUILD / "dmg"
    shutil.rmtree(folder, ignore_errors=True)
    folder.mkdir()
    subprocess.run(["ditto", str(app), str(folder / app.name)], check=True)
    (folder / "Applications").symlink_to("/Applications")  # drag the app onto it
    out.unlink(missing_ok=True)
    subprocess.run(["hdiutil", "create", "-volname", "localflow", "-srcfolder", str(folder), "-format", "UDZO",
                    "-ov", str(out)], check=True)
    return out


def msi(app, ico, version):
    out = BUILD / f"localflow-{version}-x64.msi"
    subprocess.run(["wix", "build", str(PKG / "localflow.wxs"), "-arch", "x64", "-d", f"Version={version}",
                    "-d", f"AppDir={app}", "-d", f"Icon={ico}", "-o", str(out)], check=True)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", default="0.0.1", help="x.y.z (an .msi needs numbers)")
    args = ap.parse_args()
    BUILD.mkdir(exist_ok=True)
    png, ico = icon()
    stage()
    app = pyinstaller(png, ico, args.version)
    print(f"app: {app}")
    if platform.system() == "Windows":
        print(f"installer: {msi(app, ico, args.version)}")
    elif platform.system() == "Darwin":
        print(f"installer: {dmg(app, args.version)}")


if __name__ == "__main__":
    main()
