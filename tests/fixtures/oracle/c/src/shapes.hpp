#pragma once
namespace geo {
struct Shape {
    virtual double area() const = 0;
    double describe() const;
};
}
