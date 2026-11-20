import json
import os
import pathlib
import subprocess

root=pathlib.Path(__file__).resolve().parents[1]
evidence=root/'evidence'
evidence.mkdir(exist_ok=True)
results=[]
for name,command in [('format',['cargo','fmt','--all','--check']),('rust-tests',['cargo','test','--locked']),('rust-build',['cargo','build','--locked','--bin','tezbar-brew'])]:
    result=subprocess.run(command,cwd=root,env=dict(os.environ,CARGO_BUILD_JOBS='1'),capture_output=True)
    (evidence/(name+'.log')).write_bytes(result.stdout+result.stderr)
    results.append({'check':name,'command':command,'exitCode':result.returncode})
    if result.returncode:break
(evidence/'gates.json').write_text(json.dumps(results,indent=2)+'\n')
print(json.dumps(results,indent=2))
raise SystemExit(int(any(item['exitCode'] for item in results)))
