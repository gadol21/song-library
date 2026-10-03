# PyInstaller spec for the portable desktop app. Build with build_desktop.bat.
from PyInstaller.utils.hooks import collect_all, collect_data_files, collect_submodules

datas = [
    ("app/static", "app/static"),
    ("ffmpeg/bin", "ffmpeg/bin"),
    (".env.example", "."),
]
binaries = []
hiddenimports = collect_submodules("app")

# yt-dlp loads its extractors lazily; yt-dlp-ejs carries the scripts that solve YouTube's challenges
for package in ("yt_dlp", "yt_dlp_ejs"):
    package_datas, package_binaries, package_hidden = collect_all(package)
    datas += package_datas
    binaries += package_binaries
    hiddenimports += package_hidden

datas += collect_data_files("google.genai")
hiddenimports += collect_submodules("uvicorn") + collect_submodules("google.genai")

a = Analysis(
    ["desktop.py"],
    pathex=["."],
    binaries=binaries,
    datas=datas,
    hiddenimports=hiddenimports,
    excludes=["tkinter"],
)
pyz = PYZ(a.pure)
exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name="SingAlongStudio",
    console=False,
)
coll = COLLECT(exe, a.binaries, a.datas, name="SingAlongStudio")
