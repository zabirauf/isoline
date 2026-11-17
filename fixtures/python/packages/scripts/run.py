import helper_mod
import tools.cli
from shop.core import Base, models

helper_mod.go(tools.cli.main, Base, models)
