import os
from pathlib import Path
from .util import cleanup, default, handle, ready, start, step


def classify(n):
    if n < 0:
        log("negative")
        return "neg"
    elif n == 0:
        return "zero"
    else:
        log("positive")
    return "pos" if check(n) else "big"


def scan(items, limit=10):
    for item in items:
        if not item:
            continue
        handle(item)
    else:
        log("done")
    while limit > 0 and ready():
        limit = step(limit)
    return limit


def load(path):
    try:
        with open(path) as fh:
            data = fh.read()
    except (OSError, ValueError) as err:
        log(err)
        raise RuntimeError("cannot load") from err
    else:
        log("loaded")
    finally:
        cleanup()
    return data or default()


def route(cmd):
    match cmd:
        case "start":
            start()
        case "stop" | "halt":
            return None
        case _ if cmd.startswith("x"):
            os.remove(cmd)


def log(msg):
    print(msg)


def check(n):
    return n > 100
