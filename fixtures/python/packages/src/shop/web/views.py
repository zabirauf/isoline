import os
import requests.adapters
import shop.core.models
from shop import web
from tools.cli import main

from ..core import models
from ..core.db import connect
from ..core import Base
from . import helpers
from .helpers import fmt
from ...nowhere import thing


def index():
    return fmt(models.Base(), connect, Base, helpers, os, main, web, thing)
