from limits import threshold


def main(values):
    cut = threshold(values, 3)
    return [v for v in values if v > cut]
