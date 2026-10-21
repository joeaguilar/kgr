import os
import requests
from pkg import Engine
from pkg.util import helper as h


def main():
    e = Engine()
    e.run()
    h()
    return os.getcwd()


if __name__ == "__main__":
    main()
