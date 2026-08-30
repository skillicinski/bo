module example.com/bo-consumer

go 1.27.0

require github.com/skillicinski/bo v0.0.0

require (
	cloud.google.com/go/compute/metadata v0.3.0 // indirect
	github.com/JohannesKaufmann/dom v0.2.0 // indirect
	github.com/JohannesKaufmann/html-to-markdown/v2 v2.4.0 // indirect
	golang.org/x/net v0.55.0 // indirect
	golang.org/x/oauth2 v0.36.0 // indirect
)

replace github.com/skillicinski/bo => ../..
