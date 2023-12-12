import sys

import click
import pytest


@click.command()
def main():
    sys.exit(pytest.main())

if __name__ == '__main__':
    main()
