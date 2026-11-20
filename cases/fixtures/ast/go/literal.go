package retry

// Retry settings.
const MaxAttempts = 50

type RetryOptions struct {
	Attempts int
}

func Retry(maxAttempts int, mode string) int {
	// try a few times
	label := "lenient"
	_ = label
	return maxAttempts * 2
}
