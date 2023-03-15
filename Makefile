PROJECT_NAME?=pydbsrt
#
DOCKER_USER?=yoyonel
PACKAGE_VERSION?=latest
DOCKER_TAG?=$(DOCKER_USER)/$(PROJECT_NAME):${PACKAGE_VERSION}
#
# PYPI_SERVER?=https://tart.d-bi.fr/simple/
# https://stackoverflow.com/questions/2019989/how-to-assign-the-output-of-a-command-to-a-makefile-variable
PYPI_SERVER_HOST=$(shell echo $(PYPI_SERVER) | sed -e "s/[^/]*\/\/\([^@]*@\)\?\([^:/]*\).*/\2/")
PYTEST_OPTIONS?=-v
#
TOX_DIR?=${HOME}/.tox/pydbsrt/$(PROJECT_NAME)
#
SDIST_PACKAGE=dist/${shell python setup.py --fullname}.tar.gz
SOURCES=$(shell find src/ -type f -name '*.py') MANIFEST.in

all: docker

docker: docker/Dockerfile
	@echo PYPI_SERVER: $(PYPI_SERVER)
	@docker build \
		--build-arg PYPI_SERVER=$(PYPI_SERVER) \
		-t $(DOCKER_TAG) \
		-f docker/Dockerfile \
		.

docker-run:
	@docker run --rm -it ${DOCKER_RUN_OPTIONS} $(DOCKER_TAG)

docker-run-shell:
	@docker run --rm -it ${DOCKER_RUN_OPTIONS} --entrypoint sh $(DOCKER_TAG)

pip-install:
	@pip install \
		-r requirements_dev.txt \
		--trusted-host $(PYPI_SERVER_HOST) \
		--extra-index-url $(PYPI_SERVER) \
		--upgrade

pipenv-lock:
	pipenv lock

pipenv-install_with_lock: pipenv-lock
	pipenv install --ignore-pipfile

re: fclean all

fclean:
	# https://stackoverflow.com/questions/10722723/find-exec-echo-missing-argument-to-exec
	@find . -name "*.pyc" -exec git rm --cached {} \;
	@find . -name "__pycache__" -delete

pytest:
	@poetry run pytest ${PYTEST_OPTIONS} --rootdir .

tox:
	# http://ahmetdal.org/jenkins-tox-shebang-problem/
	tox --workdir ${TOX_DIR}

db_up:	## launch DB stack
	docker-compose \
		-f docker/docker-compose.yml \
		up --remove-orphans ${DOCKERCOMPOSE_UP_OPTIONS}

db_up_detach:	## launch DB stack (in detach mode)
	DOCKERCOMPOSE_UP_OPTIONS="-d" make db_up

db_down:	## Shutdown DB stack
	docker-compose \
		-f docker/docker-compose.yml \
		down

export_imghash_from_media: data/big_buck_bunny_trailer_480p.webm	## example run for command `export_imghash_from_media`
	@poetry run pydbsrt \
  export-imghash-from-media \
  --media data/big_buck_bunny_trailer_480p.webm

/tmp/big_buck_bunny_trailer_480p.webm.25.0fps.phash: data/big_buck_bunny_trailer_480p.webm
	@make export_imghash_from_media

big_buck_bunny_trailer_480p_phash: /tmp/big_buck_bunny_trailer_480p.webm.25.0fps.phash

show-imghash-from-subtitles-and-media: 	## example run for command `show-imghash-from-subtitles-and-media`
	@poetry run pydbsrt \
		show-imghash-from-subtitles-and-media \
			--media data/big_buck_bunny_trailer_480p.webm \
			--subtitles data/SRT/big_buck_bunny_trailer_480p.en.srt

export-imghash-from-subtitles-and-media: data/big_buck_bunny_trailer_480p.webm data/SRT/big_buck_bunny_trailer_480p.en.srt
	@poetry run pydbsrt \
		show-imghash-from-subtitles-and-media \
			--media data/big_buck_bunny_trailer_480p.webm \
			--subtitles data/SRT/big_buck_bunny_trailer_480p.en.srt
	@hexdump /tmp/big_buck_bunny_trailer_480p.webm.25.0fps.phash | head

import-images-hashes-into-db: db_up_detach big_buck_bunny_trailer_480p_phash
	@poetry run pydbsrt \
		import-images-hashes-into-db \
			--binary-img-hash-file /tmp/big_buck_bunny_trailer_480p.webm.25.0fps.phash

import-subtitles-into-db: db_up_detach big_buck_bunny_trailer_480p_phash data/SRT/big_buck_bunny_trailer_480p.en.srt
	@poetry run pydbsrt \
		import-subtitles-into-db \
			--binary-img-hash-file /tmp/big_buck_bunny_trailer_480p.webm.25.0fps.phash \
			--subtitles data/SRT/big_buck_bunny_trailer_480p.en.srt

show-imghash-from-subtitles-and-media-in-db: import-images-hashes-into-db
	@poetry run pydbsrt \
		show-imghash-from-subtitles-and-media-in-db \
			--binary-img-hash-file /tmp/big_buck_bunny_trailer_480p.webm.25.0fps.phash \
			--subtitles data/SRT/big_buck_bunny_trailer_480p.en.srt

search-imghash-in-db: import-images-hashes-into-db
	@poetry run pydbsrt \
		search-imghash-in-db \
			--phash_file /tmp/big_buck_bunny_trailer_480p.webm.25.0fps.phash \
			--distance 0

default: docker
