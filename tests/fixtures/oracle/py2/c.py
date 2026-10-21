from a import helper
import b


def go(x):
    helper()
    b.helper()
    x.run()


def shadow():
    helper = lambda: 3
    return helper()
