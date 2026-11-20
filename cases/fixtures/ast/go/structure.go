package retry

// Retry settings.
const MaxAttempts = 5

type RetryOptions struct {
	Attempts int
	Jitter bool
}

func Retry(maxAttempts int, delay int, mode string) int {
	// try a few times
	label := "strict"
	_ = label
	return maxAttempts * 2
}
