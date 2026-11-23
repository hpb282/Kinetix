"""Relocate the ZIP and smoke-test with only system tools on PATH."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import zipfile

ROOT=Path(__file__).resolve().parents[1]
def main():
    archive=Path(sys.argv[1]).resolve()
    (ROOT/'.scratch').mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='Kinetix portable ',dir=ROOT/'.scratch') as directory:
        root=Path(directory)
        with zipfile.ZipFile(archive) as z:z.extractall(root)
        env=os.environ.copy();env['PATH']=str(Path(os.environ['SystemRoot'])/'System32')+';'+os.environ['SystemRoot']
        (root/'config').mkdir()
        (root/'config/native-general.json').write_text(json.dumps({'version':1,'startWithWindows':False}),encoding='utf-8')
        with (root/'smoke-test.log').open('w',encoding='utf-8') as log:
            process=subprocess.Popen([str(root/'Kinetix.exe'),'--smoke-test','--clock','--calendar','--music','--notes','--controls'],cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT)
            try:code=process.wait(timeout=30)
            except subprocess.TimeoutExpired:
                process.kill();process.wait();raise
        output=(root/'smoke-test.log').read_text(encoding='utf-8',errors='replace')
        print(output);assert code==0
        for item in ['native-dock-ready','native-clock-created','native-calendar-ready','native-music-ready','native-notes-ready','native-controls-ready']:
            assert item in output,item
        assert not (root/'runtime/node.exe').exists()
        assert not (root/'runtime/music-bridge.cjs').exists()
        print('Relocated portable smoke test passed without developer tools on PATH')
        # Python is optional. Test both entry files from an unrelated cwd, with
        # spaces in the extracted path and no developer tools available on PATH.
        for entry in ['launch_kinetix.py','Launch Kinetix.pyw']:
            launcher=subprocess.run([sys.executable,str(root/'runtime'/entry),'--smoke-test'],cwd=ROOT,env=env,capture_output=True,text=True,timeout=30)
            assert launcher.returncode==0,launcher.stderr
        assert 'native-dock-ready' in (root/'config/logs/python-launcher.log').read_text(encoding='utf-8')
        print('Optional Python and .pyw portable launchers passed')

if __name__=='__main__':main()
