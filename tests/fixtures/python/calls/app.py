from . import service
from . import utils


def main():
    data = service.fetch_users()
    cleaned = utils.normalize(data)
    utils.log("processed users")
    return cleaned


def cli():
    result = main()
    print(result)


if __name__ == "__main__":
    cli()
