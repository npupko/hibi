// Retry settings.
class Retry {
    static final int MAX_ATTEMPTS = 5;

    static class Options {
        int attempts = 3;
        boolean jitter = false;
    }

    int retry(int maxAttempts, long delay, String mode) {
        // try a few times
        String label = "strict";
        return maxAttempts * 2;
    }
}
