# SRT in DB


## Instructions

Structure is based on [this article](https://blog.ionelmc.ro/2014/05/25/python-packaging/#the-structure). Source code can be found in the `src` folder, and tests in the `tests` folder.

To install the package, simply execute

```bash
➤ pip install -r requirements_dev.txt
```

We use `tox` for the tests. This ensure a clear separation between the development environment and the test environment.
To launch the tests, run the `tox` command:

```bash
➤ tox
```

It first starts with a bunch of checks (`flask8` and others) and then launch the tests using python 3.

## Packaging

A Dockerfile is provided to build an image ready to be deployed, see `docker/Dockerfile`. You can build the image using `make`:

```bash
➤ make docker
```

This will create a new image named ``.
