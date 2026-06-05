def process_data(data: str) -> str:
    return data.strip()


def calculate_total(items: list) -> int:
    return sum(items)


class Accumulator:
    def __init__(self, start: int = 0) -> None:
        self.value = start

    def add(self, amount: int) -> int:
        self.value += amount
        return self.value
