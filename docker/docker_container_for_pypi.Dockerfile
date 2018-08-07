FROM holimetrix2/python:3.6-alpine3.6-grpc-numpy-scipy

COPY . /app

WORKDIR /app

# docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' pypi
RUN pip install \
		--trusted-host 172.17.0.2 \
		--extra-index-url http://172.17.0.2:80/simple/ \
		--no-cache-dir dist/* \
	&& \
    rm -rf ~/.cache/pip && \
    rm -rf /app

# EXPOSE 50051
# CMD ["pythie-core"]
