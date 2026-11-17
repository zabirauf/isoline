-- Shared by all repos: summaries depend only on content, prompt version and model.
CREATE TABLE IF NOT EXISTS ai_cache (
  key INTEGER PRIMARY KEY,           -- iso_core::cache::ai_key
  content_hash TEXT NOT NULL,
  prompt_version INTEGER NOT NULL,
  model TEXT NOT NULL,
  kind TEXT NOT NULL,                -- leaf | cluster | repo | file
  output_json TEXT NOT NULL,
  tokens_in INTEGER NOT NULL,
  tokens_out INTEGER NOT NULL,
  created_at INTEGER NOT NULL
);
