# Multi-stage Dockerfile powered by uv
FROM ghcr.io/astral-sh/uv:0.6.5 AS uv_bin

FROM python:3.11-slim-bookworm AS runtime
WORKDIR /app

# Install system dependencies (ffmpeg for video decoding, libpq5 for PostgreSQL)
RUN apt-get update && apt-get install -y --no-install-recommends \
    ffmpeg \
    libpq5 \
    && rm -rf /var/lib/apt/lists/*

# Install uv binary
COPY --from=uv_bin /uv /uvx /bin/

ENV UV_COMPILE_BYTECODE=1 \
    UV_LINK_MODE=copy \
    UV_PYTHON_DOWNLOADS=never \
    PATH="/app/.venv/bin:$PATH"

# Copy dependency specifications first to leverage Docker layer cache
COPY pyproject.toml uv.lock /app/

# Install dependencies using uv sync without installing root project
RUN --mount=type=cache,target=/root/.cache/uv \
    uv sync --frozen --no-install-project

# Copy application source code and install project
COPY . /app
RUN --mount=type=cache,target=/root/.cache/uv \
    uv sync --frozen

CMD ["pydbsrt"]
