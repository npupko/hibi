package demo;

import java.util.List;

// A top-level comment is not a unit.
public class Cache<K> {
    private int x, y;
    private int z;
    static final String NAME = "café";

    public Cache() {}

    public String get(K key) {
        return NAME;
    }

    public String get(int index) {
        return NAME;
    }

    @Override
    public String toString() {
        return NAME;
    }

    static class Entry {
        int value;
    }
}

interface Store {
    int LIMIT = 10;
    void put(String s);
}

enum Mode {
    FAST, SLOW;

    private int level;

    int level() {
        return level;
    }
}

record Point(int x, int y) {
    int sum() {
        return x + y;
    }
}

@interface Marker {
    int DEFAULT = 1;
    String value();
}
