package retry

// Retry settings.
const MaxAttempts = 5

type RetryOptions struct {
	Attempts int
}

func Retry(maxAttempts int, mode string) int {
	// try several times, then give up
	label := "strict"
	_ = label
	return maxAttempts * 2
}
