#
# Makefile for the CANopen Demo Node with separate build directory
#

# --- Output directories ---
BUILD_DIR = build

# The final executable name
PROJECT = canopend

# --- Compiler and Linker Settings ---
CC = gcc
CFLAGS = -Wall -g -Wno-unused-parameter
LDFLAGS = -lrt -lpthread

# --- Include Paths ---
INCLUDES = -I. \
           -ICANopenLinux \
           -ICANopenLinux/CANopenNode

# --- Source Files ---
APP_SOURCES = CO_main_basic.c \
              OD.c

DRIVER_SOURCES = CANopenLinux/CO_driver.c \
                 CANopenLinux/CO_error.c \
                 CANopenLinux/CO_epoll_interface.c \
                 CANopenLinux/CO_storageLinux.c

CANOPEN_SOURCES = $(wildcard CANopenLinux/CANopenNode/301/*.c) \
                  $(wildcard CANopenLinux/CANopenNode/303/*.c) \
                  $(wildcard CANopenLinux/CANopenNode/304/*.c) \
                  $(wildcard CANopenLinux/CANopenNode/305/*.c) \
                  $(wildcard CANopenLinux/CANopenNode/309/*.c) \
                  $(wildcard CANopenLinux/CANopenNode/storage/CO_storage.c) \
                  $(wildcard CANopenLinux/CANopenNode/extra/*.c) \
                  CANopenLinux/CANopenNode/CANopen.c

SOURCES = $(APP_SOURCES) $(DRIVER_SOURCES) $(CANOPEN_SOURCES)

# --- Object Files ---
OBJECTS = $(patsubst %.c, $(BUILD_DIR)/%.o, $(SOURCES))

# --- Build Rules ---
all: $(PROJECT)

$(PROJECT): $(OBJECTS)
	$(CC) $(CFLAGS) $(INCLUDES) $^ -o $@ $(LDFLAGS)

# Rule to compile .c to .o inside build directory, preserving folder structure
$(BUILD_DIR)/%.o: %.c
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS) $(INCLUDES) -c $< -o $@

# --- Clean Rules ---
clean:
	rm -rf $(BUILD_DIR) $(PROJECT)

deepclean: clean
	rm -rf $(BUILD_DIR)
