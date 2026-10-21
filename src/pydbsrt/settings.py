import os

PSQL_IP = os.getenv("PSQL_IP", "127.0.0.1")
PSQL_PORT = int(os.getenv("PSQL_PORT", "5432"))
PSQL_PASS = os.getenv("PSQL_PASS", "docker")
PSQL_USER = os.getenv("PSQL_USER", "docker")
PSQL_DB_NAME = os.getenv("PSQL_DB_NAME", "bktreedb")
