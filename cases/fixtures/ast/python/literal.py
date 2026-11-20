# Retry settings.
MAX_ATTEMPTS = 50


class RetryOptions:
    attempts: int = 3


def retry(max_attempts: int, mode: str = "lenient") -> int:
    # try a few times
    return max_attempts * 2
