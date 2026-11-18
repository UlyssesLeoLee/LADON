import {authenticate,requireMember,fail} from '../../../../lib/auth';
import {db} from '../../../../lib/db';
export async function POST(req:Request){
  try{
    const u=await authenticate(req);const b=await req.json();
    const {tenantId,workspaceId,conversationId,clientMessageId,bodyCiphertextBase64}=b??{};
    if (![tenantId,workspaceId,conversationId,clientMessageId,bodyCiphertextBase64].every(v=>typeof v==='string'&&v.length>0)) throw new Error('missing fields');
    await requireMember(u,tenantId,workspaceId,true);
    if(bodyCiphertextBase64.length>360_000 || !/^[A-Za-z0-9+/]*={0,2}$/.test(bodyCiphertextBase64)) throw new Error('invalid ciphertext format');
    const cipher=Buffer.from(bodyCiphertextBase64,'base64');
    if(cipher.length>256*1024 || cipher.length<24) throw new Error('ciphertext out of bounds');
    const cx=db();
    // Cross-check conversation scope and sender in one INSERT SELECT; no trust in client-provided sender ID.
    const identity=await cx.execute({sql:'SELECT id FROM identities WHERE issuer=? AND external_subject=? LIMIT 1',args:[u.issuer,u.subject]});
    const id=identity.rows[0]?.id;
    if(!id) throw new Error('forbidden');
    const messageId=crypto.randomUUID();
    const result=await cx.execute({sql:`INSERT OR IGNORE INTO chat_delivery_queue
      (id,tenant_id,workspace_id,conversation_id,sender_identity_id,client_message_id,body_ciphertext,created_at,expires_at)
      SELECT ?,c.tenant_id,c.workspace_id,c.id,?,?,?,?,? FROM conversations c
      JOIN memberships m ON m.tenant_id=c.tenant_id AND m.workspace_id=c.workspace_id
        AND m.identity_id=? AND m.state='active' AND m.role IN ('owner','admin','editor')
      WHERE c.id=? AND c.tenant_id=? AND c.workspace_id=? AND c.closed_at IS NULL`,
      args:[messageId,id,clientMessageId,cipher,new Date().toISOString(),new Date(Date.now()+7*24*3600_000).toISOString(),id,conversationId,tenantId,workspaceId]});
    if(Number(result.rowsAffected)===0) throw new Error('already_exists_or_forbidden');
    return Response.json({id:messageId,accepted:true},{status:201});
  }catch(e){return fail(e);}
}

// Optional polling relay: only nonexpired ciphertext, never plaintext.
export async function GET(req:Request){
  try{
    const u=await authenticate(req);const url=new URL(req.url);
    const tenantId=url.searchParams.get('tenantId')||'';
    const workspaceId=url.searchParams.get('workspaceId')||'';
    const conversationId=url.searchParams.get('conversationId')||'';
    await requireMember(u,tenantId,workspaceId,false);
    const rows=await db().execute({sql:`SELECT id,client_message_id,body_ciphertext,created_at FROM chat_delivery_queue
      WHERE tenant_id=? AND workspace_id=? AND conversation_id=? AND expires_at > ?
      ORDER BY created_at DESC LIMIT 100`,args:[tenantId,workspaceId,conversationId,new Date().toISOString()]});
    const items=rows.rows.map(r=>({id:r.id,clientMessageId:r.client_message_id,
      bodyCiphertextBase64:Buffer.from(r.body_ciphertext as Uint8Array).toString('base64'),createdAt:r.created_at}));
    return Response.json({items,authoritative_archive:'google_drive_or_configured_provider'});
  }catch(e){return fail(e);}
}
