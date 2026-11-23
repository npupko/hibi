# Retry settings.
MAX_TRIES = 5


class RetryOptions:
    tries: int = 3


def retry(max_tries: int, mode: str = "strict") -> int:
    # try a few times
    return max_tries * 2
