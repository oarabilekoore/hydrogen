package main

import "core:fmt"
import "core:os"

print_usage :: proc() {
	fmt.println("Usage: hydrogen <command>")
	fmt.println("")
	fmt.println("Commands:")
	fmt.println("  version     Print Hydrogen version information")
	fmt.println("")
	fmt.println("Options:")
	fmt.println("  -h, --help      Show this help")
	fmt.println("  -V, --version   Print version information")
}

main :: proc() {
	if len(os.args) <= 1 {
		print_usage()
		return
	}

	command := os.args[1]

	switch command {
	case "version", "--version", "-V":
		fmt.printfln("Hydrogen %s (Odin rewrite, experimental)", HYDROGEN_VERSION)
	case "help", "--help", "-h":
		print_usage()
	case:
		fmt.printfln("hydrogen: unknown command '%s'", command)
		fmt.println("Run 'hydrogen --help' to see available commands.")
	}
}
