enum Level {
    @Deprecated LOW,
    HIGH {
        int weight() { return 2; }
    };
    int weight() { return 1; }
}

record Pair(int left, @Nullable String right) {
    Pair {
        java.util.Objects.requireNonNull(right);
    }
    static int count;
}

interface Api {
    void call();
    int LIMIT = 3;
}

abstract class Base {
    abstract void run();
    protected int size, cap;
}
