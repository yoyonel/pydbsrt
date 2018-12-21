# -*- encoding: utf-8 -*-
from setuptools import find_packages
from setuptools import setup

# https://docs.python.org/3/distutils/setupscript.html
setup(
    setup_requires=["pytest-runner"],
    tests_require=["pytest"],
    python_requires='>=3.6'
)

