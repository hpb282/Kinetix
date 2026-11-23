"""Create a native source archive without binaries, runtimes or user settings."""
from pathlib import Path
import zipfile
ROOT=Path(__file__).resolve().parents[1]
SKIP={'.git','target','node_modules','__pycache__','bin','obj','config','.scratch'}
def main():
    output=ROOT/'share/Kinetix-native-source.zip'
    output.parent.mkdir(exist_ok=True)
    files=[ROOT/n for n in ['AGENTS.md','README.md','run_kinetix.py','run_kinetix.pyw','run_native_dock.py','run_native_clock.py','Launch Kinetix.pyw','run.cmd']]
    for folder in ['native-dock','widgets','scripts','tools','resources','docs']:
        files.extend(p for p in (ROOT/folder).rglob('*') if p.is_file() and not set(p.relative_to(ROOT).parts)&SKIP and p.suffix.lower() not in {'.exe','.dll','.pdb','.lnk','.log','.ps1'})
    with zipfile.ZipFile(output,'w',zipfile.ZIP_DEFLATED) as archive:
        for p in files: archive.write(p,'Kinetix/'+p.relative_to(ROOT).as_posix())
    print(output)
if __name__=='__main__': main()
