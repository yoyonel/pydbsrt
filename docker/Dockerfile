FROM holimetrix2/python:3.6-alpine3.6-grpc1.8.4-numpy-scipy-cbktree

COPY . /app

WORKDIR /app

ARG PYPI_SERVER=https://tart.d-bi.fr/simple/

RUN apk add --no-cache \
        gcc \
        g++ \
        libstdc++ \
        linux-headers \
        musl-dev \
    && \
				pip install \
				  --trusted-host $(echo ${PYPI_SERVER} | sed -e "s/[^/]*\/\/\([^@]*@\)\?\([^:/]*\).*/\2/") \
						--extra-index-url ${PYPI_SERVER} \
						dist/* \
				&& \
				  rm -rf ~/.cache/pip && \
						rm -rf /app \
				&& \
    		apk del --no-cache \
        gcc \
        g++ \
        linux-headers \
        musl-dev

EXPOSE 50053
ENTRYPOINT ["crawler-matching-server"]
