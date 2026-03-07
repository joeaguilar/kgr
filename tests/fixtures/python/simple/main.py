from . import utils
from . import models


def main():
    data = models.load()
    result = utils.process(data)
    print(result)


if __name__ == "__main__":
    main()
