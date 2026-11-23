# Retry settings.
MAX_ATTEMPTS=5

class RetryOptions :
    attempts : int=3


def retry(max_attempts: int,
          mode: str="strict") -> int:
    # try a few times
    return max_attempts*2
