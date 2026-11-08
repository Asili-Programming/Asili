# ESP-IDF project for the habari example: build the Asili side first (`pata jenga --lengo esp32`
# in the example's folder), then `idf.py build flash monitor` here.
cmake_minimum_required(VERSION 3.16)
include($ENV{IDF_PATH}/tools/cmake/project.cmake)
project(habari)
