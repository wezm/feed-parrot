SCDOC:=scdoc
PREFIX?=/usr/local
MANDIR?=$(PREFIX)/share/man
DESTDIR?=

all: feed-parrot.1

feed-parrot.1: feed-parrot.1.scd
	$(SCDOC) < feed-parrot.1.scd > feed-parrot.1

install:
	mkdir -p $(DESTDIR)$(MANDIR)/man1
	install -m644 feed-parrot.1 $(DESTDIR)$(MANDIR)/man1/feed-parrot.1

clean:
	rm -f feed-parrot.1

