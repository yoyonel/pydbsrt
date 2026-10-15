#!/bin/bash

set -e

# Perform all actions as $POSTGRES_USER
export PGUSER="$POSTGRES_USER"

# Create the 'bktreedb' db
echo "Create bktreedb DATABASE"
psql --user "$PGUSER" --db bktreedb <<-'EOSQL'
CREATE DATABASE bktreedb;
EOSQL

# Enable extension in current database (Note: This is per-database, so if you want to use it on
# multiple DBs, you'll have to enable it in each.
echo "Enable bktree EXTENSION"
psql --user "${PGUSER}" --db bktreedb <<-'EOSQL'
CREATE EXTENSION IF NOT EXISTS bktree CASCADE;
EOSQL
