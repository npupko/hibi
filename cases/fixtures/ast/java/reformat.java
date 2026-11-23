// Retry settings.
class Retry
{
  static   final int MAX_ATTEMPTS   =   5;

  static class Options
  {
      int attempts=3;
  }

  int retry(int maxAttempts,
            String mode)
  {
      // try a few times
      String label="strict";
      return maxAttempts*2;
  }
}
