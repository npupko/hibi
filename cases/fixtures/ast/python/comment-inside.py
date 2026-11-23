# Retry settings.
MAX_ATTEMPTS = 5


class RetryOptions:
    attempts: int = 3


def retry(max_attempts: int, mode: str = "strict") -> int:
    # try several times, then give up
    return max_attempts * 2
