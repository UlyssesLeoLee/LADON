import { createClient } from '@tursodatabase/serverless/compat';
// Backend control-plane ONLY: tenant membership, device identity, chat metadata.
// No document contents or Drive OAuth refresh tokens live here.
export function db() {
  const url=process.env.TURSO_DATABASE_URL;
  const authToken=process.env.TURSO_AUTH_TOKEN;
  if (!url || !authToken) throw new Error('Turso control plane is not configured');
  return createClient({url,authToken});
}
