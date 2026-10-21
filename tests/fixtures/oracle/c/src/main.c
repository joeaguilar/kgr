#include <stdio.h>
#include "../include/geo/point.h"
#include "local.h"

int main(void) {
    Point a = {0, 0}, b = {1, 1};
    printf("%d\n", point_dist(a, b));
    return 0;
}
