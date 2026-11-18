import {authenticate,fail} from '../../../../../lib/auth';
export async function GET(req:Request){
  try{const user=await authenticate(req);return Response.json({issuer:user.issuer,subject:user.subject,control_plane:'vercel',storage:'user_owned_drive'});}
  catch(e){return fail(e);}
}
