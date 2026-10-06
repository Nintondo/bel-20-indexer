"""Exercise deployment/rollback against real Docker Compose on an isolated runner.

The tiny fixture images test Compose, health transitions, includes and backup
restoration. The separate image-build job validates the actual application.
"""
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]
FIXTURE = json.loads(Path(__file__).with_name('deployment-fixture.json').read_text())[0]

def run(*args, **kwargs):
    return subprocess.run(args, check=True, text=True, **kwargs)

SERVER = r'''
const http=require('http');
const blocked=(process.env.MAINTENANCE_NETWORKS||'').split(',').filter(Boolean);
const normalized=blocked.map(n=>n.replace(/-(mainnet|testnet)$/,(_,s)=>s[0].toUpperCase()+s.slice(1)));
http.createServer((q,r)=>{
 if(q.url==='/readyz'||q.url==='/api/readyz'){
  if(process.env.FORCE_UNHEALTHY==='true'){r.writeHead(503);r.end('unhealthy');return;}
  r.setHeader('Content-Type','application/json');r.end(JSON.stringify({status:'ok',maintenanceNetworks:normalized}));return;
 }
 if(q.url==='/api/electrs/blocks'&&normalized.includes(q.headers['x-network'])){r.writeHead(503,{'Content-Type':'application/json'});r.end(JSON.stringify({error:'NETWORK_MAINTENANCE'}));return;}
 if(blocked.some(n=>q.url===`/${n.replace('-mainnet','')}/explorer`)){r.writeHead(307,{location:'/maintenance'});r.end();return;}
 r.end('fixture');
}).listen(Number(process.env.PORT),'0.0.0.0');
'''

def main():
    suffix=uuid.uuid4().hex[:10]
    registry='deploy-test-registry-'+suffix
    service_name=FIXTURE['compose_service']
    project='deploy-test-'+suffix
    container="deploy-fixture-"+suffix
    with tempfile.TemporaryDirectory(dir=os.environ.get("RUNNER_TEMP")) as temporary:
        root=Path(temporary)
        service=root/FIXTURE['service_path'].lstrip('/')
        base=root/FIXTURE['base_path'].lstrip('/')
        service.mkdir(parents=True); base.mkdir(parents=True,exist_ok=True)
        target=service/FIXTURE['compose_file']
        root_compose=root/FIXTURE['root_compose'].lstrip('/')
        context=root/'image'; context.mkdir(); (context/'server.js').write_text(SERVER)
        # Registry is local to this disposable runner; no project registry login.
        run('docker','run','-d','--name',registry,'-p','127.0.0.1::5000','registry:2',stdout=subprocess.DEVNULL)
        try:
            mapping=run('docker','port',registry,'5000/tcp',capture_output=True).stdout.strip()
            registry_host=mapping
            for _ in range(30):
                # The runner may be a container using a host Docker socket.
                # Probe inside the registry, not the runner's own loopback.
                p=subprocess.run(['docker','exec',registry,'wget','-q','-O','/dev/null','http://127.0.0.1:5000/v2/'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
                if not p.returncode: break
                time.sleep(1)
            else: raise RuntimeError('Local registry did not start')
            env=dict(os.environ,**FIXTURE['env'])
            env.update(CI_REGISTRY=registry_host,CI_REGISTRY_REPO='fixture',DEPLOYMENT_ID='compose_test',DEPLOY_PHASE='deploy',HEALTH_TIMEOUT_SECONDS='30',RELOAD_NGINX='false',COMPOSE_PROJECT_NAME=project,COMPOSE_SERVICE_NAME_OVERRIDE=service_name)
            if env.get('SERVICE_PATH_INPUT'): env['SERVICE_PATH_INPUT']=str(service)
            image_name=env['IMAGE_NAME_OVERRIDE']
            old=f'{registry_host}/fixture/{image_name}:'+ 'a'*40
            new=f'{registry_host}/fixture/{image_name}:'+ 'b'*40
            port={'tokens':8000,'content':8111,'electrs':3001,'faucet':7373,'inscriber':3002,'market':9043,'pusher':8002,'screenshoter':3000,'frontend':3000}.get(env['SERVICE_NAME'],3002)
            for tag,image,health in [('a',old,False),('b',new,True)]:
                dockerfile='FROM node:22-alpine\nWORKDIR /app\nCOPY server.js /app/server.js\n'
                # Node fixture CLI validates real docker exec/stdin plumbing.
                dockerfile+="RUN for name in bells-cli dogecoin-cli pepecoin-cli; do printf '#!/bin/sh\\nexit 0\\n' > /app/$name; chmod +x /app/$name; done\n"
                dockerfile+="RUN printf '#!/bin/sh\\necho fixture -g"+'b'*40+"\\n' > /app/bellsd && chmod +x /app/bellsd\n"
                dockerfile+=f'ENV PORT={port}\nLABEL org.opencontainers.image.revision="'+tag*40+'"\n'
                if health:
                    dockerfile+='HEALTHCHECK --interval=1s --timeout=2s --retries=1 CMD node -e "fetch(\'http://127.0.0.1:\'+process.env.PORT+\'/readyz\').then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))"\n'
                dockerfile+='CMD ["node","/app/server.js"]\n'
                (context/'Dockerfile').write_text(dockerfile)
                run('docker','build','--quiet','-t',image,str(context),stdout=subprocess.DEVNULL)
                run('docker','push','--quiet',image,stdout=subprocess.DEVNULL)

            def compose_text(image):
                return f'services:\n  {service_name}:\n    container_name: {container}\n    image: {image}\n    environment:\n      PORT: "{port}"\n'

            target.write_text(compose_text(old))
            if root_compose!=target:
                # Real include paths resolve relative to the project file.
                root_compose.write_text('include:\n  - '+str(target)+'\n')
            originals={target:target.read_bytes()}
            for relative in FIXTURE.get('files',[]):
                p=service/relative; p.parent.mkdir(parents=True,exist_ok=True); p.write_text('CI_VALUE=original\n'); originals[p]=p.read_bytes()
            for relative in FIXTURE.get('staged_files',[]):
                p=service/relative; p.parent.mkdir(parents=True,exist_ok=True)
                p.write_text(compose_text(new) if relative.endswith(('.yml.updated','.yaml.updated')) else 'CI_VALUE=updated\n')
            script=root/'deploy.sh'
            text=(ROOT/'.github/actions/deploy-over-ssh/deploy.sh').read_text().replace('/opt/',str(root/'opt')+'/')
            # Keep logical Compose service names, but never use a production
            # container name on a shared self-hosted Docker daemon.
            text,count=re.subn(r'(?m)^CONTAINER=.*$',lambda _: 'CONTAINER="'+container+'"',text)
            assert count==1, 'Expected one container assignment'
            text=text.replace("'services': {os.environ['CONTAINER']: {", "'services': {"+repr(service_name)+": {")
            script.write_text(text)
            run('docker','compose','-f',str(root_compose),'up','-d','--no-deps',service_name,env=env,stdout=subprocess.DEVNULL)
            run('bash',str(script),env=env)
            assert (service/'.deploy-transaction').is_dir()
            assert run('docker','inspect','-f','{{.Config.Image}}',container,capture_output=True).stdout.strip()==new
            # A CI-side failure after SSH returns must still restore old image/files.
            env['DEPLOY_PHASE']='rollback'; run('bash',str(script),env=env)
            assert run('docker','inspect','-f','{{.Config.Image}}',container,capture_output=True).stdout.strip()==old
            for p,contents in originals.items(): assert p.read_bytes()==contents,str(p)
            assert (service/'.deploy-transaction/restored').is_file()
            print('Real Compose deploy and legacy recovery passed:',service_name)
        finally:
            subprocess.run(['docker','rm','-f',container,registry],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            subprocess.run(['docker','network','rm',project+'_default'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)

if __name__=='__main__':
    main()
