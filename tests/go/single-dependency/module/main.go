package main

import (
	"fmt"	
	"go-module/hello"
)

const FLOX = "flox"

func main() {
	helloFlox := hello.Hello(FLOX)

	// Say hello to flox.
	fmt.Println(helloFlox)
}
