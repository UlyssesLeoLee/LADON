import {authenticate,fail} from '../../../../lib/auth';
import {db} from '../../../../lib/db';
export async function GET(req:Request){
  try{
    const u=await authenticate(req);const cx=db();
    const r=await cx.execute({sql:`SELECT w.id,w.tenant_id,w.name,m.role FROM memberships m
      JOIN identities i ON i.id=m.identity_id
      JOIN workspaces w ON w.id=m.workspace_id AND w.tenant_id=m.tenant_id
      WHERE i.issuer=? AND i.external_subject=? AND m.state='active'
      ORDER BY w.name LIMIT 200`,args:[u.issuer,u.subject]});
    return Response.json({items:r.rows});
  }catch(e){return fail(e)}
}
