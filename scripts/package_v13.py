#!/usr/bin/env python3
from pathlib import Path
import hashlib,json,zipfile,datetime
root=Path(__file__).resolve().parents[1]
output=root.parent/'LADON_Explorer_IPA_V1_3_最新版完整包.zip'
exclude={'PACKAGE_MANIFEST.json','SHA256SUMS.txt'}
files=sorted(p for p in root.rglob('*') if p.is_file() and p.relative_to(root).as_posix() not in exclude and '__pycache__' not in p.parts and 'node_modules' not in p.parts)
manifest={'product':'LADON Explorer','version':'1.3.0','date':'2026-11-18','baseline':'Design and source candidate, no production acceptance','requirements':100,'nonfunctional_requirements':43,'test_specs':{'UT':55,'IT':30,'ST':74},'svg_count':20,'interactive_html':'ui-v13-responsive/interactive-preview.html','cloud_control_plane':'optional Vercel for team; not live','rust_build':'not run','cloud_oauth':'not run','windows_shell_test':'not run','files':[]}
for p in files:
    h=hashlib.sha256(p.read_bytes()).hexdigest();manifest['files'].append({'path':p.relative_to(root).as_posix(),'bytes':p.stat().st_size,'sha256':h})
(root/'PACKAGE_MANIFEST.json').write_text(json.dumps(manifest,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
checksum=[f"{f['sha256']}  {f['path']}" for f in manifest['files']]
checksum.append(f"{hashlib.sha256((root/'PACKAGE_MANIFEST.json').read_bytes()).hexdigest()}  PACKAGE_MANIFEST.json")
(root/'SHA256SUMS.txt').write_text('\n'.join(checksum)+'\n',encoding='utf-8')
allfiles=sorted(p for p in root.rglob('*') if p.is_file() and '__pycache__' not in p.parts and 'node_modules' not in p.parts)
with zipfile.ZipFile(output,'w',zipfile.ZIP_DEFLATED,compresslevel=6) as z:
    for p in allfiles:z.write(p,arcname='LADON_Explorer_IPA_V1_3/'+p.relative_to(root).as_posix())
with zipfile.ZipFile(output) as z:
    bad=z.testzip()
    assert bad is None,bad
    assert len(z.namelist())==len(allfiles)
print(json.dumps({'archive':str(output),'file_count':len(allfiles),'archive_bytes':output.stat().st_size,'sha256':hashlib.sha256(output.read_bytes()).hexdigest()},ensure_ascii=False,indent=2))
