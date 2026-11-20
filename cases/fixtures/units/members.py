class Config:
    name = "x"
    port: int = 80
    debug: bool
    handler = lambda: 1

    def load(self):
        local = 1
        return local

if True:
    def hidden():
        pass

TOP = 1
