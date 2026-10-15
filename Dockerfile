FROM python:3.9-bookworm as base
ENV PIP_DISABLE_PIP_VERSION_CHECK=1
WORKDIR /app

FROM base as poetry
RUN pip install poetry==1.4.2
COPY poetry.lock pyproject.toml /app/
RUN poetry export --with dev --without-hashes -o requirements.txt

FROM base as build
COPY --from=poetry /app/requirements.txt /tmp/requirements.txt
RUN python -m venv .venv && \
    .venv/bin/pip install 'wheel==0.36.2' && \
    .venv/bin/pip install -r /tmp/requirements.txt && \
    .venv/bin/pip install sh

FROM python:3.9-slim-bookworm as runtime
ENV PIP_DISABLE_PIP_VERSION_CHECK=1
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends ffmpeg && rm -rf /var/lib/apt/lists/*
ENV PATH=/app/.venv/bin:$PATH
ENV PYTHONPATH=/app/src:$PYTHONPATH
COPY --from=build /app/.venv /app/.venv
COPY . /app
RUN .venv/bin/pip install --no-deps -e .