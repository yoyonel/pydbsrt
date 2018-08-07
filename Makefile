PROJECT_NAME?=pydbsrt
#
PACKAGE_NAME=$(shell python setup.py --name)
PACKAGE_FULLNAME=$(shell python setup.py --fullname)
PACKAGE_VERSION:=$(shell python setup.py --version)
#
DOCKER_TAG?=dailycodingproblem/$(PROJECT_NAME):${PACKAGE_VERSION}
#
# PYPI_SERVER?=https://tart.d-bi.fr/simple/
# https://stackoverflow.com/questions/2019989/how-to-assign-the-output-of-a-command-to-a-makefile-variable
PYPI_SERVER_HOST=$(shell echo $(PYPI_SERVER) | sed -e "s/[^/]*\/\/\([^@]*@\)\?\([^:/]*\).*/\2/")
PYTEST_OPTIONS?=-v
#
TOX_DIR?=${HOME}/.tox/pydbsrt/$(PROJECT_NAME)
#
SDIST_PACKAGE=dist/${shell python setup.py --fullname}.tar.gz
SOURCES=$(shell find src/ -type f -name '*.py') setup.py MANIFEST.in

all: docker

${SDIST_PACKAGE}: ${SOURCES}
	@echo "Building python project..."
	@python setup.py sdist

docker: ${SDIST_PACKAGE}
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

pypi-register:
	python setup.py register -r pydbsrt
	
pypi-upload: pypi-register
	python setup.py sdist upload -r pydbsrt

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
	pytest ${PYTEST_OPTIONS}

tox:
	# http://ahmetdal.org/jenkins-tox-shebang-problem/
	tox --workdir ${TOX_DIR}

default: docker
