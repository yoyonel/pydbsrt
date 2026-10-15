FROM postgis/postgis:9.6-3.0
LABEL maintainer="BKTreeDB Project"
ENV POSTGRES_DBNAME=bktreedb
ENV POSTGRES_USER=docker 
ENV POSTGRES_PASSWORD=docker
RUN apt-get update \
    && apt-get install -y \
    build-essential \
    cmake \
    git \
    libpq-dev \
    liblwgeom-dev \
    libproj-dev \
    libjson-c-dev \
    postgresql-server-dev-9.6
RUN git clone https://github.com/yoyonel/pg-spgist_hamming -b develop /usr/local/src/pg-spgist_hamming
# RUN mkdir /usr/local/src/pg-spgist_hamming/build
RUN cd /usr/local/src/pg-spgist_hamming/bktree && \
    make && \
    make install

RUN rm /docker-entrypoint-initdb.d/10_postgis.sh
COPY ./initdb-bktree.sh /docker-entrypoint-initdb.d/
RUN chmod +x /docker-entrypoint-initdb.d/initdb-bktree.sh
