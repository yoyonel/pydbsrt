# Stage 1: Build bktree extension
FROM postgres:17-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    git \
    postgresql-server-dev-17 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN git clone https://github.com/yoyonel/pg-spgist_hamming -b develop /usr/local/src/pg-spgist_hamming \
    && cd /usr/local/src/pg-spgist_hamming/bktree \
    && make \
    && make install

# Stage 2: Clean PostgreSQL 17 runtime
FROM postgres:17-bookworm

LABEL maintainer="BKTreeDB Project"

# Copy compiled extension libraries and SQL definitions
COPY --from=builder /usr/lib/postgresql/17/lib/bktree.so /usr/lib/postgresql/17/lib/
COPY --from=builder /usr/share/postgresql/17/extension/bktree* /usr/share/postgresql/17/extension/

# Copy initialization script
COPY ./initdb-bktree.sh /docker-entrypoint-initdb.d/initdb-bktree.sh
RUN chmod +x /docker-entrypoint-initdb.d/initdb-bktree.sh
