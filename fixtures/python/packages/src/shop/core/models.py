from .db import connect


class Base:
    def save(self):
        connect().write(self)
