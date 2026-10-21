import functools


class Engine:
    class Inner:
        def inner_m(self):
            return 1

    @staticmethod
    def build():
        return Engine()

    @functools.lru_cache
    def run(self):
        return self.step()

    def step(self):
        return 2


def helper():
    return "core"


def fact(n):
    return 1 if n <= 1 else n * fact(n - 1)
