"""Shapes module."""
from __future__ import annotations

import math
import os.path
import numpy as np
from typing import TYPE_CHECKING, Optional
from .util import clamp as _clamp, scale

if TYPE_CHECKING:
    from .models import Model

__all__ = ["Circle", "area", "MAX_RADIUS"]

MAX_RADIUS = 100
_DEFAULT = 1.0


def area(r: float) -> float:
    """Area of a circle."""
    return math.pi * _clamp(r, 0, MAX_RADIUS) ** 2


@dataclass
class Circle(Base):
    """A circle.

    Has a radius.
    """

    radius: float = _DEFAULT

    def grow(self, by=1, *args, **kw) -> Circle:
        def bump(x):
            return scale(x, by)

        self.radius = bump(self.radius)
        return self

    @property
    async def load(self, model: Optional[Model] = None):
        data = await np.load(os.path.join("a", "b"))
        return data


def helper():
    return area
