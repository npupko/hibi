package retry

// Retry settings.
const MaxTries = 5

type RetryOptions struct {
	Tries int
}

func Retry(maxTries int, mode string) int {
	// try a few times
	label := "strict"
	_ = label
	return maxTries * 2
}
