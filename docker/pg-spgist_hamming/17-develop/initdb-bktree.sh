#!/bin/bash
set -e

# Enable bktree in default database
psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" <<-EOSQL
    CREATE EXTENSION IF NOT EXISTS bktree CASCADE;
EOSQL

# Enable bktree in template1 so any test databases automatically inherit the extension
psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname template1 <<-EOSQL
    CREATE EXTENSION IF NOT EXISTS bktree CASCADE;
EOSQL
