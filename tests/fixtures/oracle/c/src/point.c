#include "geo/point.h"
#include <stdlib.h>

static int sq(int v) { return v * v; }

int point_dist(Point a, Point b) {
    return sq(a.x - b.x) + sq(a.y - b.y);
}
