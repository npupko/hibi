// Retry settings.
class Retry {
    static final int MAX_TRIES = 5;

    static class Options {
        int tries = 3;
    }

    int retry(int maxTries, String mode) {
        // try a few times
        String label = "strict";
        return maxTries * 2;
    }
}
