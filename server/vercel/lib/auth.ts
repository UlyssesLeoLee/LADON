import {createRemoteJWKSet,jwtVerify} from 'jose';
import {db} from './db';
const keys=createRemoteJWKSet(new URL('https://www.googleapis.com/oauth2/v3/certs'));
export interface Identity { issuer:string; subject:string; email?:string; }
export async function authenticate(request:Request):Promise<Identity> {
  const token=request.headers.get('Authorization')?.match(/^Bearer (\S+)$/)?.[1];
  if (!token) throw new Error('unauthenticated');
  const audience=process.env.GOOGLE_OIDC_CLIENT_ID;
  if (!audience) throw new Error('server OIDC configuration missing');
  const {payload}=await jwtVerify(token,keys,{issuer:['https://accounts.google.com','accounts.google.com'],audience});
  if (typeof payload.sub!=='string') throw new Error('subject missing');
  return {issuer:'https://accounts.google.com',subject:payload.sub,email:typeof payload.email==='string'?payload.email:undefined};
}
export async function requireMember(id:Identity,tenantId:string,workspaceId:string,write:boolean){
  if (!tenantId || !workspaceId) throw new Error('tenant and workspace required');
  const cx=db();
  const record=await cx.execute({sql:`SELECT m.role FROM memberships m
    JOIN identities i ON i.id=m.identity_id
    JOIN workspaces w ON w.id=m.workspace_id AND w.tenant_id=m.tenant_id
    WHERE i.issuer=? AND i.external_subject=? AND m.tenant_id=? AND m.workspace_id=? AND m.state='active'
    LIMIT 1`,args:[id.issuer,id.subject,tenantId,workspaceId]});
  const role=record.rows[0]?.role;
  if (!role || (write && !['owner','admin','editor'].includes(String(role)))) throw new Error('forbidden');
  return String(role);
}
export function fail(error:unknown){
  const m=error instanceof Error?error.message:'unknown';
  const code=(m==='unauthenticated'||m==='subject missing'||m.includes('JWT')||m.includes('signature')||m.includes('expired'))?401:m==='forbidden'?403:m==='already_exists_or_forbidden'?409:m==='missing fields'||m==='invalid ciphertext format'||m==='ciphertext out of bounds'?400:500;
  // Do not echo environment secrets or DB exception details.
  return Response.json({error:code===500?'internal_error':code===400?'request_invalid':code===409?'conflict':m},{status:code});
}
