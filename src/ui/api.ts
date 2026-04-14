export interface Stream {
  id: string;
  title: string;
  summary: string;
  branch: string;
  created_at: string;
  updated_at: string;
  panes: { working: string; talking: string };
}

export async function getCurrentStream(): Promise<Stream> {
  const r = await fetch("/api/streams/current");
  if (!r.ok) throw new Error(`failed: ${r.status}`);
  return r.json();
}
