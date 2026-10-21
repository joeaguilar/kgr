#include <vector>
#include "shapes.hpp"

namespace geo {
template <typename T>
T twice(T v) { return v + v; }

class Circle : public Shape {
public:
    double area() const override { return 3.14 * r * r; }
    double r = 1;
};
}

int run() {
    geo::Circle c;
    return (int)geo::twice(c.area());
}
