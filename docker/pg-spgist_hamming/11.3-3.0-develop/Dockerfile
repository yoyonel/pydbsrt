FROM postgis/postgis:11-3.3

LABEL maintainer="BKTreeDB Project"
ENV POSTGRES_DBNAME=bktreedb
ENV POSTGRES_USER=docker
ENV POSTGRES_PASSWORD=docker

RUN sed -i 's/^# deb/deb/g' /etc/apt/sources.list \
    && sed -i 's/^deb http:\/\/deb.debian.org/# deb http:\/\/deb.debian.org/g' /etc/apt/sources.list \
    && sed -i 's|http://apt.postgresql.org|https://apt-archive.postgresql.org|g' /etc/apt/sources.list.d/pgdg.list \
    && echo 'Acquire::Check-Valid-Until "false";' > /etc/apt/apt.conf.d/99no-check-valid-until \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
    build-essential \
    git \
    postgresql-server-dev-11 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/local/src
RUN git clone https://github.com/yoyonel/pg-spgist_hamming -b develop /usr/local/src/pg-spgist_hamming \
    && make -C /usr/local/src/pg-spgist_hamming/bktree \
    && make -C /usr/local/src/pg-spgist_hamming/bktree install

RUN rm /docker-entrypoint-initdb.d/10_postgis.sh
COPY ./initdb-bktree.sh /docker-entrypoint-initdb.d/
RUN chmod +x /docker-entrypoint-initdb.d/initdb-bktree.sh
