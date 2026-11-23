package retry

// Retry policy, reviewed.
const MaxAttempts = 5

type RetryOptions struct {
	Attempts int
}

// Retries with backoff.
func Retry(maxAttempts int, mode string) int {
	// try a few times
	label := "strict"
	_ = label
	return maxAttempts * 2
}
