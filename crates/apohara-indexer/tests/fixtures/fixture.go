package fixture

// Repo holds an accumulated total.
type Repo struct {
	total int
}

// ProcessData is a free function.
func ProcessData(data string) string {
	return data
}

// CalculateTotal sums a slice of ints.
func CalculateTotal(items []int) int {
	sum := 0
	for _, item := range items {
		sum += item
	}
	return sum
}

// Add is a method on *Repo (it carries a receiver).
func (r *Repo) Add(amount int) int {
	r.total += amount
	return r.total
}
