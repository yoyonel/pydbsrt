ARG APPLICATION_NAME=pydbsrt
ARG POETRY_VERSION=1.1.12

FROM python:3.9-slim as base
ENV PIP_DISABLE_PIP_VERSION_CHECK=1
WORKDIR /${APPLICATION_NAME}
RUN python -m pip install --upgrade pip==20.2.4 && \
    echo '***VERSION python in builder image' && \
    python --version


FROM base as poetry
COPY poetry.lock pyproject.toml /${APPLICATION_NAME}/
RUN pip install poetry==1.1.12 && \
    poetry export -o requirements.txt


FROM base as build
COPY --from=poetry /${APPLICATION_NAME}/requirements.txt /tmp/requirements.txt
# https://stackoverflow.com/questions/62715570/failing-to-install-psycopg2-binary-on-new-docker-container
RUN apt-get update && \
    apt-get -y install libpq-dev gcc && \
    python -m venv .venv && \
    .venv/bin/pip install 'wheel==0.36.2' && \
    .venv/bin/pip install -r /tmp/requirements.txt

#FROM base as runtime
##ADD . /${APPLICATION_NAME}
#ENV PATH=/app/.venv/bin:$PATH
#COPY --from=build /app/.venv /app/.venv
#COPY . /app