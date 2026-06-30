PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;

CREATE TABLE IF NOT EXISTS pr (
  number INTEGER PRIMARY KEY,
  title TEXT NOT NULL,
  author TEXT NOT NULL,
  author_type TEXT NOT NULL,
  state TEXT NOT NULL,
  merged_at TEXT NOT NULL,
  base_sha TEXT,
  head_sha TEXT,
  merge_commit_sha TEXT,
  url TEXT NOT NULL,
  fetched_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS pr_file (
  pr_number INTEGER NOT NULL REFERENCES pr(number) ON DELETE CASCADE,
  path TEXT NOT NULL,
  status TEXT,
  additions INTEGER,
  deletions INTEGER,
  PRIMARY KEY (pr_number, path)
);
CREATE INDEX IF NOT EXISTS idx_pr_file_path ON pr_file(path);

CREATE TABLE IF NOT EXISTS pr_comment (
  id INTEGER PRIMARY KEY,
  pr_number INTEGER NOT NULL REFERENCES pr(number) ON DELETE CASCADE,
  author TEXT NOT NULL,
  author_type TEXT NOT NULL,
  body TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_pr_comment_pr ON pr_comment(pr_number);
CREATE INDEX IF NOT EXISTS idx_pr_comment_author ON pr_comment(author);

CREATE TABLE IF NOT EXISTS reviewer (
  login TEXT PRIMARY KEY,
  weight REAL NOT NULL,
  tier INTEGER NOT NULL,
  notes TEXT
);

CREATE TABLE IF NOT EXISTS review_summary (
  id INTEGER PRIMARY KEY,
  pr_number INTEGER NOT NULL REFERENCES pr(number) ON DELETE CASCADE,
  author TEXT NOT NULL,
  author_type TEXT NOT NULL,
  state TEXT NOT NULL,
  body TEXT,
  submitted_at TEXT NOT NULL,
  commit_id TEXT
);
CREATE INDEX IF NOT EXISTS idx_review_summary_pr ON review_summary(pr_number);
CREATE INDEX IF NOT EXISTS idx_review_summary_author ON review_summary(author);

CREATE TABLE IF NOT EXISTS line_comment (
  id INTEGER PRIMARY KEY,
  pr_number INTEGER NOT NULL REFERENCES pr(number) ON DELETE CASCADE,
  author TEXT NOT NULL,
  author_type TEXT NOT NULL,
  created_at TEXT NOT NULL,
  path TEXT NOT NULL,
  line INTEGER,
  original_line INTEGER,
  side TEXT,
  diff_hunk TEXT,
  body TEXT NOT NULL,
  is_noise INTEGER NOT NULL DEFAULT 0,
  in_reply_to_id INTEGER,
  area TEXT NOT NULL,
  reviewer_weight REAL NOT NULL DEFAULT 1.0,
  reviewer_tier INTEGER NOT NULL DEFAULT 3,
  commit_id TEXT,
  original_commit_id TEXT,
  thread_resolved INTEGER NOT NULL DEFAULT 0,
  thread_resolved_by TEXT,
  thread_resolved_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_comment_area ON line_comment(area);
CREATE INDEX IF NOT EXISTS idx_comment_author ON line_comment(author);
CREATE INDEX IF NOT EXISTS idx_comment_pr ON line_comment(pr_number);
CREATE INDEX IF NOT EXISTS idx_comment_tier ON line_comment(reviewer_tier);
CREATE INDEX IF NOT EXISTS idx_comment_noise ON line_comment(is_noise);

CREATE TABLE IF NOT EXISTS comment_final_code (
  comment_id INTEGER PRIMARY KEY REFERENCES line_comment(id) ON DELETE CASCADE,
  final_code_snippet TEXT,
  snippet_available INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS classification (
  comment_id INTEGER PRIMARY KEY REFERENCES line_comment(id) ON DELETE CASCADE,
  taxonomy TEXT NOT NULL,
  was_addressed INTEGER,
  rule_statement TEXT,
  confidence REAL NOT NULL,
  classifier_model TEXT NOT NULL,
  classified_at TEXT NOT NULL,
  raw_response TEXT,
  prompt_hash TEXT
);
CREATE INDEX IF NOT EXISTS idx_classification_taxonomy ON classification(taxonomy);
CREATE INDEX IF NOT EXISTS idx_classification_addressed ON classification(was_addressed);

CREATE TABLE IF NOT EXISTS finding (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  theme TEXT NOT NULL,
  rule_statement TEXT NOT NULL,
  taxonomy TEXT NOT NULL,
  area TEXT NOT NULL,
  scope TEXT NOT NULL CHECK (scope IN ('cross-cutting','area-specific')),
  primary_reviewer TEXT,
  reviewer_logins TEXT NOT NULL,
  tier1_reviewer_count INTEGER NOT NULL,
  tier2_reviewer_count INTEGER NOT NULL,
  total_evidence_count INTEGER NOT NULL,
  evidence_comment_ids TEXT NOT NULL,
  evidence_pr_numbers TEXT NOT NULL,
  cross_area_count INTEGER NOT NULL,
  areas_seen TEXT NOT NULL,
  acceptance_rate REAL,
  in_agents_md INTEGER NOT NULL,
  agents_md_section TEXT,
  confidence_score REAL NOT NULL,
  notes TEXT,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_finding_area ON finding(area);
CREATE INDEX IF NOT EXISTS idx_finding_scope ON finding(scope);
CREATE INDEX IF NOT EXISTS idx_finding_confidence ON finding(confidence_score DESC);

CREATE TABLE IF NOT EXISTS synthesis_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  kind TEXT NOT NULL CHECK (kind IN ('cross_cutting','area','gap','other')),
  area TEXT,
  model TEXT NOT NULL,
  prompt_hash TEXT NOT NULL,
  system_prompt TEXT NOT NULL,
  user_prompt TEXT NOT NULL,
  raw_response TEXT NOT NULL,
  output_path TEXT,
  written_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_synthesis_log_kind ON synthesis_log(kind);
CREATE INDEX IF NOT EXISTS idx_synthesis_log_written ON synthesis_log(written_at);
