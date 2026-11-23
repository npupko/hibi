"""Module docstring is a statement."""
import os
from typing import List

# A top-level comment is not a unit.
GREETING = "héllo"
LIMIT: int = 10
x = y = 0
a, b = 1, 2


def helper():
    return os.sep


def helper():
    return "again"


@decorator
@other(1)
def wrapped(n):
    def inner():
        return n
    return inner


class Cache(object):
    size = 0

    def get(self, key):
        return key

    @staticmethod
    def make():
        return Cache()

    class Entry:
        value: int


@dataclass
class Point:
    x: int = 0
    names: List[str] = []


if __name__ == "__main__":
    def local():
        pass
    print(helper())
