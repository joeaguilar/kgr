from .. import base
from ..base import callback
import proj.base as pb


def use():
    base.shared()
    pb.shared()
    return list(map(callback, [1]))
