DEVPORT=1507
DEVFILE="dev"
PRODPORT=48228
PRODFILE="prod"

MANIFEST=../Cargo.toml

.PHONY: local global
local: surnames.txt names.txt ipaddr.txt
	echo '"localhost"' > ipaddr.txt
	mkdir -p dev
	cargo run $(DEVPORT) $(DEVFILE)
	 
global: surnames.txt names.txt ipaddr.txt
	make -B ipaddr.txt
	mkdir -p prod
	cargo run --release $(PRODPORT) $(PRODFILE)

surnames.txt: 
	curl https://raw.githubusercontent.com/Hyneman/moby-project/672f6bdca054c42d375f065ffee87e8ceba0c242/moby/mwords/21986na.mes |\
	iconv -f cp1252 - | grep -v " " > surnames.txt

names.txt:
	curl https://raw.githubusercontent.com/Hyneman/moby-project/672f6bdca054c42d375f065ffee87e8ceba0c242/moby/mwords/3897male.nam > male.txt
	curl https://raw.githubusercontent.com/Hyneman/moby-project/672f6bdca054c42d375f065ffee87e8ceba0c242/moby/mwords/4946fema.len > female.txt
	cat male.txt female.txt | sort | uniq | grep -v " " > names.txt

ipaddr.txt:
	dig +short @dns.toys -4 ip > ipaddr.txt
