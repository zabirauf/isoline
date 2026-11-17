import json


def outer(data, *args, **kw):
    def helper(x):
        return json.dumps(x)

    class Local:
        def method(self):
            return helper(1)

    pairs = [(a, b) for a in data for b in args if a != b]
    callbacks = [lambda: helper(p) for p in pairs]
    try:
        emit(helper(pairs))
    finally:
        close()
    return callbacks


async def stream(source):
    async with source.lock() as guard:
        async for chunk in source.read():
            yield transform(chunk)
    while True:
        if source.done():
            break
