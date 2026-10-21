package com.acme.app;

import com.acme.core.Repo;
import static com.acme.core.Util.twice;

public class Main {
    public static void main(String[] args) {
        Repo r = new Repo();
        r.count("x");
        twice(2);
    }
}
