#!/usr/bin/env sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
WINDIE_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
DB_PATH="$HOME/.windie/windie.db"

if [ ! -d "$WINDIE_DIR" ]; then
  echo "windie repository not found at $WINDIE_DIR" >&2
  exit 1
fi

mkdir -p "$(dirname -- "$DB_PATH")"

if [ -f "$DB_PATH" ]; then
  BACKUP_PATH="$DB_PATH.backup.$(date +%Y%m%d%H%M%S)"
  mv -- "$DB_PATH" "$BACKUP_PATH"
  echo "moved $DB_PATH to $BACKUP_PATH"
else
  echo "no existing database at $DB_PATH"
fi

(
  cd "$WINDIE_DIR"
  cargo run --quiet -- ls >/dev/null
)

if [ ! -f "$DB_PATH" ]; then
  echo "failed to recreate database at $DB_PATH" >&2
  exit 1
fi

if command -v sqlite3 >/dev/null 2>&1; then
  VERSION=$(sqlite3 "$DB_PATH" "PRAGMA user_version;")
  echo "created $DB_PATH with schema version $VERSION"
else
  echo "created $DB_PATH"
fi
