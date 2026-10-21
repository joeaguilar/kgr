package com.acme.core;

import java.util.List;

public class Repo {
    public static class Entry {
        public String key() { return "k"; }
    }

    public List<Entry> all() { return List.of(); }

    public int count() { return 0; }
    public int count(String filter) { return count(); }

    private void hidden() {}
}
