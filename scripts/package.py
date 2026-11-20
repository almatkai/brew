"""Package a local Windows *development* executable, preserving macOS-only policy.
No artifact URLs or public release pins are created. License copies come from the
exact Cargo.lock-resolved source directories, including target-specific crates.
"""
import hashlib
import argparse
import json
import pathlib
import subprocess
import stat
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--target',default='x86_64-pc-windows-msvc',choices=['x86_64-pc-windows-msvc','aarch64-apple-darwin'])
parser.add_argument('--binary',type=pathlib.Path)
parser.add_argument('--output',type=pathlib.Path,default=ROOT/'dist')
options=parser.parse_args()
target=options.target
executable='bin/brew.exe' if 'windows' in target else 'bin/brew'
binary=options.binary or (ROOT/'target/debug'/('tezbar-brew.exe' if 'windows' in target else 'tezbar-brew'))
SOURCE = "b1e7745b664f9cde82b6e671fc760e8488501a05"
SDK = "e955911f0bf5b558f9d3eb5183df0adc95c22af2"
digest = lambda b: hashlib.sha256(b).hexdigest()
dist = options.output
dist.mkdir(parents=True,exist_ok=True)
upstream = json.loads((ROOT / "reference/package.json").read_text(encoding="utf-8"))
commands = []
for source in upstream["commands"]:
    command = dict(source)
    command.update(platforms=["macOS"], runtime="rust", concurrency="serialize", refreshMs=None, timeoutMs=300000)
    commands.append(command)
manifest = json.loads((ROOT/'extension.json').read_text(encoding='utf-8'))
assert manifest['commands']==commands and manifest['preferences']==upstream['preferences']
assert manifest['schemaVersion']==3 and manifest['runtime']=='rust' and manifest['execution']=='process'
assert manifest['platforms']==['macOS'] and manifest['id']=='com.tezbar.native.brew'
info=json.loads(subprocess.check_output([str(binary.resolve()),'--protocol-info'],timeout=15))
assert info['id']==manifest['id'] and info['version']==manifest['version'] and info['protocol']==1
assert set(info['commands'])=={command['name']for command in commands}
header=binary.read_bytes()
if 'windows' in target:
    assert header[:2]==b'MZ'
    offset=int.from_bytes(header[60:64],'little')
    assert header[offset:offset+6]==b'PE\0\0d\x86','Expected Windows x64 executable'
else:
    assert header[:4]==bytes.fromhex('cffaedfe') and int.from_bytes(header[4:8],'little')==0x0100000c,'Expected Apple Silicon Mach-O'
manifest['artifacts']={}
metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", target], cwd=ROOT))
files = {executable: binary.read_bytes(),
         "notices/raycast-brew/LICENSE": (ROOT / "reference/LICENSE").read_bytes(),
         "provenance/package.json": (ROOT / "reference/package.json").read_bytes(),
         "provenance/Cargo.lock": (ROOT / "Cargo.lock").read_bytes()}
for asset in (ROOT / "reference/assets").rglob("*"):
    if asset.is_file():
        files["resources/" + asset.relative_to(ROOT / "reference/assets").as_posix()] = asset.read_bytes()
licenses = []
for package in metadata["packages"]:
    if package["id"] == metadata["resolve"]["root"]:
        continue
    source_root = pathlib.Path(package["manifest_path"]).parent
    copied = []
    for item in source_root.iterdir():
        if item.is_file() and any(item.name.upper().startswith(prefix) for prefix in ("LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT")):
            name = "notices/" + package["name"] + "-" + package["version"] + "/" + item.name
            files[name] = item.read_bytes()
            copied.append(name)
    if package.get("license_file"):
        item = source_root / package["license_file"]
        if item.is_file():
            name = "notices/" + package["name"] + "-" + package["version"] + "/" + item.name
            files[name] = item.read_bytes()
            copied.append(name)
    if not copied:
        raise RuntimeError("Missing dependency license notice: " + package["id"])
    licenses.append({"name":package["name"], "version":package["version"], "license":package["license"], "source":package["source"], "files":sorted(set(copied))})
files["notices/dependencies.json"] = (json.dumps(licenses, indent=2) + "\n").encode()
files["provenance/source.json"] = (json.dumps({"raycastCommit":SOURCE, "sdkCommit":SDK, "developmentTarget":target, "declaredPlatforms":["macOS"], "productionRelease":False},indent=2)+"\n").encode()
manifest["files"] = {name:digest(content) for name,content in sorted(files.items())}
archive = dist / ("brew-"+target+"-development.zip")
with zipfile.ZipFile(archive,"w",zipfile.ZIP_DEFLATED) as z:
    payload=dict(files,**{'extension.json':(json.dumps(manifest,indent=2)+'\n').encode()})
    for name,content in sorted(payload.items()):
        entry=zipfile.ZipInfo(name,date_time=(2026,1,1,0,0,0))
        entry.create_system=3
        entry.external_attr=(stat.S_IFREG|(0o755 if name==executable else 0o644))<<16
        entry.compress_type=zipfile.ZIP_DEFLATED
        z.writestr(entry,content)
with zipfile.ZipFile(archive) as z:
    assert set(z.namelist())==set(files)|{'extension.json'}
    assert all(digest(z.read(name))==sha for name,sha in manifest['files'].items())
    assert (z.getinfo(executable).external_attr>>16)&0o111,'Packaged executable lost execute permission'
result = {"archive":str(archive),"sha256":digest(archive.read_bytes()),"executableSha256":manifest["files"][executable],"sourceCommit":SOURCE,"sdkCommit":SDK,"developmentOnly":True,"target":target,"declaredPlatforms":["macOS"],"dependencyNotices":len(licenses),"files":len(files)}
archive.with_suffix(".zip.sha256").write_text(result["sha256"]+"\n")
(dist / "artifact.json").write_text(json.dumps(result,indent=2)+"\n")
(dist / (archive.name+'.artifact.json')).write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result,indent=2))
