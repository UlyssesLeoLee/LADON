#!/usr/bin/env python3
"""Static IPA baseline QA. Does NOT replace cargo, live Drive, Vercel or Windows ST."""
from pathlib import Path
import json,sqlite3,xml.etree.ElementTree as ET,yaml,zipfile,hashlib,re
root=Path(__file__).resolve().parents[1]
report={'baseline':'LADON V1.3','date':'2026-11-18','checks':{},'executed_test_level': 'Static-only','rust_compile':'Not Run: cargo unavailable','remote_oauth':'Not Run','windows_shell':'Not Run'}
def check(name,fn):
    try:
        value=fn();report['checks'][name]={'status':'pass','detail':value}
    except Exception as e:
        report['checks'][name]={'status':'fail','error':str(e)}
def dload(p):return json.loads((root/p).read_text(encoding='utf-8'))
def basics():
    req=dload('design/requirements.json');nfr=dload('design/nfrs.json');tc=dload('design/testcases.json')
    assert len(req)==100 and len(nfr)==43
    assert [a['id'] for a in req]==[f'REQ-{i:03}' for i in range(1,101)]
    assert len(tc['UT'])==55 and len(tc['IT'])==30 and len(tc['ST'])==74
    for kind,seq in [('UT',55),('IT',30),('ST',74)]:
        assert [a[0] for a in tc[kind]]==[f'{kind}-{i:03}' for i in range(1,seq+1)]
    cases={a[0] for v in tc.values() for a in v}
    assert all(r['main_test'] in cases for r in req)
    return {'functional':len(req),'nfr':len(nfr),'ut':55,'it':30,'st':74}
check('requirements_and_traces',basics)
def sqlcheck():
    con=sqlite3.connect(':memory:')
    for p in sorted((root/'sql').glob('*.sql')):
        con.executescript(p.read_text(encoding='utf-8'))
    names={r[0] for r in con.execute("select name from sqlite_master where type='table'")}
    needed=['resources','versions','events','identities','tenants','workspaces','chat_delivery_queue','archive_event_segments','sync_merge_conflicts','storage_rule_sets']
    assert all(x in names for x in needed),[x for x in needed if x not in names]
    return {'tables':len(names),'migrations':3}
check('sql_sqlite_compatibility',sqlcheck)
def schemas():
    a=yaml.safe_load((root/'api/ladon-agent-openapi.yaml').read_text(encoding='utf-8'))
    b=yaml.safe_load((root/'protocol/drive-layout-v1.yaml').read_text(encoding='utf-8'))
    assert a['openapi']=='3.1.0' and len(a['paths'])>=60
    assert 'team_shared' in b['modes'] and 'personal_private' in b['modes']
    assert 'cannot_share' in b['modes']['personal_private']['limits']
    assert b['sync']['conflict'].find('NEVER last-write-wins')>=0
    return {'openapi_paths':len(a['paths']),'storage_modes':list(b['modes'])}
check('contracts_and_storage_protocol',schemas)
def svg_check():
    a=list((root/'ui-modern-svg').glob('*.svg'))
    b=list((root/'ui-v13-responsive').glob('*.svg'))
    for p in a+b:ET.parse(p)
    assert len(a)==14 and len(b)==6
    assert (root/'ui-v13-responsive/interactive-preview.html').exists()
    return {'legacy_svg':len(a),'new_svg':len(b),'interactive_html':True}
check('svg_xml_and_ui_assets',svg_check)
def codecheck():
    for f in ['src/cloud_journal.rs','src/storage_rules.rs','src/ui_layout.rs','src/emergence.rs','server/vercel/lib/auth.ts','server/vercel/app/api/v1/chat/route.ts']:
        assert (root/f).stat().st_size>500
    txt=(root/'src/cloud_journal.rs').read_text()
    assert 'sensitive journal cannot be published' in txt
    assert 'readback' in txt
    txt=(root/'server/vercel/lib/auth.ts').read_text()
    assert 'jwtVerify' in txt and "m.state='active'" in txt
    return 'source files present; content spot-check only (not compilation)'
check('code_presence_security_spot_checks',codecheck)
def workbookcheck():
    p=root/'docs/04_tests/要求追踪_测试管理.xlsx'
    with zipfile.ZipFile(p) as z:
        assert z.testzip() is None
        workbook=z.read('xl/workbook.xml').decode('utf-8')
        assert len(ET.fromstring(workbook).find('{http://schemas.openxmlformats.org/spreadsheetml/2006/main}sheets'))==11
    return {'bytes':p.stat().st_size,'sheets':11}
check('xlsx_zip_structure',workbookcheck)
if __name__=='__main__':
    ok=all(v['status']=='pass' for v in report['checks'].values())
    report['overall_static']='PASS' if ok else 'FAIL'
    (root/'QA_REPORT.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(json.dumps(report,ensure_ascii=False,indent=2))
    raise SystemExit(0 if ok else 1)
