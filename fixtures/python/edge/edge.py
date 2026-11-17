import a, b.c as bc
from . import sibling
from ..pkg.mod import y as z
from x import *

type Alias = list[Widget]
VERSION: str = "1.0"


class Widget:
    def __init__(self, a):
        self._a = a

    @property
    def size(self):
        return self._a

    @size.setter
    def size(self, value):
        self._a = value


@app.route("/")
def index(sibling):
    return sibling.render(z(Widget()))


def _private():
    class Inner(Widget):
        pass
    return Inner


main = lambda: index(None)
index(VERSION)
