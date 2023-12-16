surnames.txt: 
	curl https://raw.githubusercontent.com/Hyneman/moby-project/672f6bdca054c42d375f065ffee87e8ceba0c242/moby/mwords/21986na.mes |\
	iconv -f cp1252 - > surnames.txt

names.txt:
	curl https://raw.githubusercontent.com/Hyneman/moby-project/672f6bdca054c42d375f065ffee87e8ceba0c242/moby/mwords/3897male.nam > male.txt
	curl https://raw.githubusercontent.com/Hyneman/moby-project/672f6bdca054c42d375f065ffee87e8ceba0c242/moby/mwords/4946fema.len > female.txt
	cat male.txt female.txt | sort | uniq > names.txt
