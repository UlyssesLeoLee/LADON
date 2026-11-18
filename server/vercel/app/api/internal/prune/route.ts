import {db} from '../../../../lib/db';
// Vercel Cron: optional, authenticates using a dedicated CRON_SECRET in Authorization.
export async function GET(request:Request){
  const secret=process.env.CRON_SECRET;
  if (!secret || request.headers.get('authorization')!==`Bearer ${secret}`){
    return Response.json({error:'unauthorized'},{status:401});
  }
  try {
    const result=await db().execute({sql:'DELETE FROM chat_delivery_queue WHERE expires_at < ?',args:[new Date().toISOString()]});
    return Response.json({expiredMessagesPruned:Number(result.rowsAffected)});
  } catch {return Response.json({error:'prune_failed'},{status:500})}
}
