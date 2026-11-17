import asyncio
import numpy as np
from .store import Store, fetch


class Cache(Store):
    def __init__(self, size=8):
        super().__init__()
        self.size = size

    @property
    def full(self):
        return len(self.items) >= self.size

    async def refresh(self, keys, on_done=None):
        rows = [self.load(k) for k in keys if k]
        values = await asyncio.gather(*rows)
        if on_done:
            on_done(values)
        return {k: np.mean(v) for k, v in zip(keys, values)}

    def load(self, key):
        return fetch(key)


def summarize(rows, key=lambda r: r.id):
    def score(row):
        return weight(row) * 2

    ranked = sorted(rows, key=score)
    names = list(map(lambda r: r.name.upper(), ranked))
    later = lambda: notify(names)
    report = lambda: render(ranked)
    print(report())
    key(rows[0])
    return names


def weight(row):
    return row.size or 1
