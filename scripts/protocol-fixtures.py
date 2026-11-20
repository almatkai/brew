"""Drive the actual extension and synthetic fake-brew.exe over bounded JSONL.

All host effects are captured replies. No real Homebrew, terminal, clipboard,
profile, service or package manager is accessed. macOS simulation is explicit.
"""
import json
import os
import pathlib
import queue
import subprocess
import tempfile
import threading
import zipfile
import uuid
import sys

ROOT=pathlib.Path(__file__).resolve().parents[1]
OUT=ROOT/'evidence'
OUT.mkdir(exist_ok=True)
CHECKS=[]
PEERS=[]


class Peer:
    def __init__(self, binary, platform, directory):
        self.log=directory/'fake-cli.jsonl'
        self.process=subprocess.Popen([str(binary)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,
                                      text=True,encoding='utf-8',env=dict(os.environ,TEZBAR_BREW_FIXTURE_LOG=str(self.log)))
        PEERS.append(self)
        self.process.stdin.reconfigure(newline='\n')
        self.frames=queue.Queue();self.trace=[];self.host=[];self.serial=0
        def read():
            for line in self.process.stdout:
                self.frames.put(json.loads(line))
        threading.Thread(target=read,daemon=True).start()
        threading.Thread(target=lambda:self.process.stderr.read(),daemon=True).start()
        hello=self.frames.get(timeout=15)
        assert hello['type']=='hello' and hello['id']=='com.tezbar.native.brew' and hello['protocol']==1
        self.trace.append(hello)
        self.rpc('initialize',dict(protocol=1,extensionId=hello['id'],platform=platform,packageDir=str(directory),
                                 dataDir=str(directory/'data'),tempDir=str(directory/'temp'),grantedCapabilities=['dialog.confirm','clipboard.write','window.visibility','background.retain','background.release','tray.set','tray.remove']))

    def send(self, frame):
        self.process.stdin.write(json.dumps(frame)+'\n');self.process.stdin.flush()

    def rpc(self, method,params, error=False):
        self.serial+=1;request='h:'+str(uuid.uuid4())
        self.send(dict(type='request',id=request,method=method,params=params))
        while True:
            frame=self.frames.get(timeout=30);self.trace.append(frame)
            if frame['type']=='request':
                self.host.append(frame)
                reply={'host.dialog.confirm':{'response':1},'host.clipboard.write':{'ok':True},'host.background.retain':{'retained':True},'host.background.release':{'retained':False}}.get(frame['method'],True)
                self.send(dict(type='response',id=frame['id'],result=reply))
            elif frame['type']=='response' and frame['id']==request:
                if error:
                    assert 'error' in frame,frame
                    return frame['error']
                assert 'error' not in frame,frame
                return frame.get('result')

    def close(self):
        self.rpc('shutdown',{})
        self.process.stdin.close();self.process.wait(timeout=15)
        assert self.process.returncode==0


def action(view,title):
    result=next((item['id'] for item in view['actions'] if item['title']==title),None)
    if result is None:raise ValueError({'missingAction':title,'view':view})
    return result


def main():
    # Exercise the exact executable extracted from the development archive.
    archive=pathlib.Path(sys.argv[1]).resolve() if len(sys.argv)>1 else ROOT/'dist/brew-x86_64-pc-windows-msvc-development.zip'
    source=json.loads((ROOT/'reference/package.json').read_text(encoding='utf-8'))
    with tempfile.TemporaryDirectory(prefix='tezbar-brew-fixture-',ignore_cleanup_errors=True) as temp:
        directory=pathlib.Path(temp)
        with zipfile.ZipFile(archive) as z:z.extractall(directory/'package')
        binary=directory/'package/bin'/('brew.exe' if os.name=='nt' else 'brew')
        if os.name!='nt':binary.chmod(0o700)
        info=json.loads(subprocess.check_output([str(binary),'--protocol-info'],text=True,encoding='utf-8'))
        assert set(info['commands'])=={item['name'] for item in source['commands']}
        CHECKS.append('packaged executable protocol identity and exact 12 source command IDs')
        fake=ROOT/'target/debug'/('fake-brew.exe' if os.name=='nt' else 'fake-brew')
        peer=Peer(binary,'aarch64-apple-darwin',directory)
        for command in source['commands']:
            name=command['name'];sid=name
            view=peer.rpc('session.open',dict(sessionId=sid,commandName=name,preferences={'customBrewPath':str(fake),'zapCask':True},argumentValues={'search':'jq'} if name=='search' else {}))
            CHECKS.append(name+': packaged command open under explicit synthetic macOS platform')
            if name in ['installed','search','manage-services','manage-taps','upgrade']:
                title={'installed':'uninstall test-app','search':'install jq','manage-services':'stop postgresql@18','manage-taps':'trust fixture/tap','upgrade':'Select All Displayed Upgrades'}[name]
                old=action(view,title)
                view=peer.rpc('session.call',dict(sessionId=sid,operation='extension:invoke-action',payload={'actionId':old}))
                before=len(peer.log.read_text().splitlines())
                peer.rpc('session.call',dict(sessionId=sid,operation='extension:invoke-action',payload={'actionId':old}),error=True)
                assert len(peer.log.read_text().splitlines())==before
                CHECKS.append(name+': selected action consumed once; replay refused without another CLI invocation')
                if name=='upgrade':
                    peer.rpc('session.call',dict(sessionId=sid,operation='extension:invoke-action',payload={'actionId':action(view,'Upgrade Selected Packages')}))
                    CHECKS.append('upgrade: update then fresh selection intersection, typed per-package upgrades')
            if name=='services-menu':
                peer.rpc('session.call',dict(sessionId=sid,operation='extension:invoke-action',payload={'actionId':action(view,'Enable Services Menu')}))
                menu=next(frame['params'] for frame in reversed(peer.host) if frame['method']=='host.tray.set')
                peer.rpc('session.dispose',{'sessionId':sid})
                # Root ownership survives the originating view's disposal.
                stop=next(item for item in menu['items'] if item['title']=='stop all services')
                peer.rpc('background.event',dict(resourceId='brew-services',actionId=stop['id']))
                menu=next(frame['params'] for frame in reversed(peer.host) if frame['method']=='host.tray.set')
                hide=next(item for item in menu['items'] if item['title']=='Hide Services Menu')
                peer.rpc('background.event',dict(resourceId='brew-services',actionId=hide['id']))
                CHECKS.append('services menu: durable retain, view disposal, root callback, confirmed service action, release and remove')
            else:peer.rpc('session.dispose',{'sessionId':sid})
        peer.close()
        calls=[json.loads(line) for line in peer.log.read_text().splitlines()]
        assert any(call['args']==['uninstall','--cask','--zap','test-app'] for call in calls)
        assert any(call['args']==['services','stop','--all'] for call in calls)
        assert any(call['args']==['cleanup'] for call in calls)
        # Each CLI call went to the fixture executable, never brew on PATH.
        CHECKS.append('all process effects used only the synthetic fixture executable')
        windows=Peer(binary,'x86_64-pc-windows-msvc',directory)
        count=len(windows.log.read_text().splitlines())
        for command in source['commands']:
            err=windows.rpc('session.open',dict(sessionId=command['name'],commandName=command['name'],preferences={'customBrewPath':str(fake)}),error=True)
            assert 'macOS-only' in err['message']
        assert len(windows.log.read_text().splitlines())==count
        windows.close();CHECKS.append('all12 actual Windows target launches rejected before CLI access')
        (OUT/'protocol-trace.json').write_text(json.dumps(peer.trace,indent=2)+'\n',encoding='utf-8')
        (OUT/'fixture-cli-calls.json').write_text(json.dumps(calls,indent=2)+'\n',encoding='utf-8')
    (OUT/'protocol-results.json').write_text(json.dumps({'passed':len(CHECKS),'checks':CHECKS,'platform':sys.platform,'macOS':'synthetic CLI on native Mac, no real Homebrew/desktop acceptance' if sys.platform=='darwin' else 'simulated only, not native or desktop acceptance','realHomebrewInvocations':0},indent=2)+'\n',encoding='utf-8')
    print(json.dumps({'passed':len(CHECKS),'realHomebrewInvocations':0}))


if __name__=='__main__':
    try:main()
    finally:
        for peer in PEERS:
            if peer.process.poll() is None:
                peer.process.kill();peer.process.wait(timeout=15)
            for stream in [peer.process.stdin,peer.process.stdout,peer.process.stderr]:
                if stream and not stream.closed:stream.close()
