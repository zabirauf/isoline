-- Find the few unresolved imports without scanning every import (re-resolution after files
-- are added or removed).
CREATE INDEX IF NOT EXISTS imports_unresolved ON imports(file_id) WHERE unresolved = 1;
