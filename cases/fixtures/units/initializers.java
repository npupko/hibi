class Init {
    static int n;
    static {
        n = 1;
    }
    {
        n = 2;
    }
    int m;
}

enum Level {
    HIGH {
        int weight = 3;
        int w() { return weight; }
    },
    LOW;
}
